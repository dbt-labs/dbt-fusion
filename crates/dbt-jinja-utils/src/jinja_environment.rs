use dbt_common::{fs_err, ErrorCode, FsError, FsResult};
use dbt_fusion_adapter::{
    factory::create_static_relation, BaseAdapter, BridgeAdapter, ParseAdapter, SqlEngine,
};
use minijinja::{
    listener::RenderingEventListener,
    value::{mutable_map::MutableMap, ValueMap},
    Environment, Error as MinijinjaError, State, Template, UndefinedBehavior, Value,
};
use serde::Serialize;
use std::{borrow::Cow, collections::BTreeMap, rc::Rc, sync::Arc};
use tracy_client::span;

/// A struct that wraps a Minijinja Expression.
///
/// This is to consolidate the Minijinja::Error to FsError conversion
/// where ever we invokes directly a method from a minijinja::Expression instance in a scope that we need to return a FsResult
pub struct JinjaExpression<'env, 'source>(minijinja::Expression<'env, 'source>);

impl<'env: 'source, 'source> JinjaExpression<'env, 'source> {
    /// Evaluate the expression
    pub fn eval<S: Serialize>(
        &self,
        ctx: S,
        listeners: &[Rc<dyn RenderingEventListener>],
    ) -> FsResult<Value> {
        let result = self.0.eval(ctx, listeners).map_err(|e| {
            FsError::from_jinja_err(e, "Failed to eval the compiled Jinja expression")
        })?;
        Ok(result)
    }
}

/// A struct that wraps a Minijinja Template.
///
/// This is to consolidate the Minijinja::Error to FsError conversion
/// where ever we invokes directly a method from a minijinja::Template instance in a scope that we need to return a FsResult
pub struct JinjaTemplate<'env, 'source>(Template<'env, 'source>);

impl<'env: 'source, 'source> JinjaTemplate<'env, 'source> {
    /// Evaluates the template into a state
    pub fn eval_to_state<S: Serialize>(
        &self,
        ctx: S,
        listeners: &[Rc<dyn RenderingEventListener>],
    ) -> FsResult<State<'_, '_>> {
        let result = self
            .0
            .eval_to_state(ctx, listeners)
            .map_err(|e| FsError::from_jinja_err(e, "Failed to render the Jinja template"))?;
        Ok(result)
    }
}

/// A struct that wraps a Minijinja Environment.
#[derive(Clone)]
pub struct JinjaEnvironment<'source> {
    env: Environment<'source>,
    sql_engine: Option<Arc<SqlEngine>>,
}

impl<'a> AsRef<JinjaEnvironment<'a>> for JinjaEnvironment<'a> {
    fn as_ref(&self) -> &JinjaEnvironment<'a> {
        self
    }
}

impl<'source> JinjaEnvironment<'source> {
    /// Create a new JinjaEnvironment.
    pub fn new(env: Environment<'source>) -> Self {
        Self {
            env,
            sql_engine: None,
        }
    }

    /// Create a new empty state.
    pub fn empty_state(&self) -> State<'_, '_> {
        self.env.empty_state()
    }

    /// Create a new state with a pre-interned string map.
    pub fn new_state_with_context(&self, ctx: BTreeMap<String, Value>) -> State<'_, '_> {
        self.env.new_state_with_context(MutableMap::from(
            ctx.into_iter()
                .map(|(k, v)| (Value::from(k), v))
                .collect::<ValueMap>(),
        ))
    }

    /// Render a template from a string.
    pub fn render_str<S: Serialize>(
        &self,
        source: &str,
        ctx: S,
        listeners: &[Rc<dyn RenderingEventListener>],
    ) -> FsResult<String> {
        let _span = span!("render_str");
        let result = self
            .env
            .render_str(source, ctx, listeners)
            .map_err(|e| FsError::from_jinja_err(e, "Failed to render the Jinja str"))?;
        Ok(result)
    }

    /// Render named template from a string.
    pub fn render_named_str<S: Serialize>(
        &self,
        name: &str,
        source: &str,
        ctx: S,
        listeners: &[Rc<dyn RenderingEventListener>],
    ) -> Result<String, MinijinjaError> {
        self.env.render_named_str(name, source, ctx, listeners)
    }

    /// Get a reference to the stored [SqlEngine], if available.
    pub fn sql_engine(&self) -> Option<&Arc<SqlEngine>> {
        self.sql_engine.as_ref()
    }

    /// Adds a global variable.
    pub fn add_global<N, V>(&mut self, name: N, value: V)
    where
        N: Into<Cow<'source, str>>,
        V: Into<Value>,
    {
        self.env.add_global(name, value);
    }

    /// Get a global variable.
    pub fn get_global(&self, name: &str) -> Option<Value> {
        self.env.get_global(name)
    }

    /// Remove a global variable.
    pub(crate) fn remove_global(&mut self, name: &str) {
        self.env.remove_global(name);
    }

    /// Add a function to the environment.
    pub(crate) fn add_function<N, F, Rv, Args>(&mut self, name: N, f: F)
    where
        N: Into<Cow<'source, str>>,
        // the crazy bounds here exist to enable borrowing in closures
        F: minijinja::functions::Function<Rv, Args>
            + for<'a> minijinja::functions::Function<
                Rv,
                <Args as minijinja::value::FunctionArgs<'a>>::Output,
            >,
        Rv: minijinja::value::FunctionResult,
        Args: for<'a> minijinja::value::FunctionArgs<'a>,
    {
        self.env.add_function(name, f);
    }

    /// Compile an expression.
    pub fn compile_expression(&self, expr: &'source str) -> FsResult<JinjaExpression<'_, 'source>> {
        Ok(JinjaExpression(self.env.compile_expression(expr).map_err(
            |e| FsError::from_jinja_err(e, "Failed to compile Jinja expression"),
        )?))
    }

    /// Set the adapter
    pub(crate) fn set_adapter(&mut self, adapter: Arc<dyn BaseAdapter>) {
        let mut api_map = BTreeMap::new();
        api_map.insert(
            "Relation".to_string(),
            create_static_relation(adapter.adapter_type(), adapter.quoting()),
        );
        api_map.insert("Column".to_string(), adapter.column_type());
        self.env.add_global("api", Value::from_object(api_map));

        // Add the adapter type to the environment for easy access
        self.sql_engine = adapter.engine().cloned();
        self.env
            .add_global("dialect", Value::from(adapter.adapter_type().to_string()));
        self.env.add_global("adapter", adapter.as_value());
    }

    /// Get the adapter from the environment
    pub fn get_base_adapter(&self) -> Option<Arc<dyn BaseAdapter>> {
        let adapter = self.env.get_global("adapter")?;
        let bridge = adapter.downcast_object::<BridgeAdapter>()?;
        Some(bridge as Arc<dyn BaseAdapter>)
    }

    /// Get the parse adapter from the environment
    pub fn get_parse_adapter(&self) -> Option<Arc<ParseAdapter>> {
        let adapter = self.env.get_global("adapter")?;
        adapter.downcast_object::<ParseAdapter>()
    }

    /// Set the undefined behavior for the environment.
    pub(crate) fn set_undefined_behavior(&mut self, behavior: UndefinedBehavior) {
        self.env.set_undefined_behavior(behavior);
    }

    /// Check if a template exists.
    pub fn has_template(&self, name: &str) -> bool {
        self.env.get_template(name).is_ok()
    }

    /// Get a template from the environment.
    pub fn get_template(&self, name: &str) -> FsResult<JinjaTemplate> {
        if !self.has_template(name) {
            return Err(fs_err!(
                ErrorCode::JinjaError,
                "Template not found: {}",
                name
            ));
        }
        let result = self
            .env
            .get_template(name)
            .map_err(|e| FsError::from_jinja_err(e, "Failed to get template"))?;
        Ok(JinjaTemplate(result))
    }

    /// Get the dbt and adapters namespace.
    pub fn get_dbt_and_adapters_namespace(&self) -> Arc<ValueMap> {
        self.env.get_dbt_and_adapters_namespace()
    }

    /// Get the target context.
    pub fn get_target_context(&self) -> Arc<BTreeMap<String, String>> {
        self.env
            .get_global("target")
            .unwrap_or_default()
            .downcast_object::<BTreeMap<String, String>>()
            .unwrap_or_default()
    }
}
