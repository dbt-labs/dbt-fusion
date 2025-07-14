// cfg.rs
use crate::compiler::instructions::Instruction;
use std::collections::BTreeSet;

pub type BlockId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    FallThrough,
    Uncond,
    Cond(bool),
}

#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: BlockId,
    pub start: usize,
    pub end: usize,
    pub successor: Vec<(BlockId, EdgeKind)>,
    pub predecessor: Vec<BlockId>,
    pub current_macro: Option<String>,
    pub type_constraints: std::collections::BTreeMap<String, crate::types::builtin::Type>,
}

#[derive(Debug, Clone)]
pub struct CFG {
    pub blocks: Vec<BasicBlock>,
    instruction_to_basic_block: Vec<BlockId>,
    pub entry: BlockId,
}

impl CFG {
    #[inline]
    pub fn block_of(&self, inst_idx: usize) -> BlockId {
        self.instruction_to_basic_block[inst_idx]
    }

    pub fn successor(&self, bb: BlockId) -> &[(BlockId, EdgeKind)] {
        &self.blocks[bb].successor
    }

    pub fn predecessor(&self, bb: BlockId) -> &[BlockId] {
        &self.blocks[bb].predecessor
    }

    pub fn instructions<'code>(
        &self,
        bb: BlockId,
        code: &'code [Instruction<'code>],
    ) -> &'code [Instruction<'code>] {
        let b = &self.blocks[bb];
        &code[b.start..=b.end]
    }

    pub fn to_dot(&self) -> String {
        let mut s = String::from("digraph cfg {  node [shape=box];");
        for b in &self.blocks {
            s.push_str(&format!("  B{} [label=\"B{}\"];", b.id, b.id));
            for (succ, kind) in &b.successor {
                s.push_str(&format!("  B{} -> B{} [label=\"{:?}\"];", b.id, succ, kind));
            }
        }
        s.push('}');
        s
    }

    pub fn dump_blocks(&self, code: &[Instruction]) -> String {
        let mut ret = String::new();
        for block in &self.blocks {
            ret.push_str(&format!(
                "Block B{} ({}..={}):\n",
                block.id, block.start, block.end
            ));

            for (offset, inst) in code[block.start..=block.end].iter().enumerate() {
                ret.push_str(&format!("  {:>4}: {:?}\n", block.start + offset, inst));
            }
            ret.push('\n');
        }
        ret
    }

    /// return a reference to the block with the given id
    pub fn get_block(&self, id: BlockId) -> Option<&BasicBlock> {
        self.blocks.get(id)
    }
}

fn is_block_terminator(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::Jump(_)
            | Instruction::JumpIfFalse(_)
            | Instruction::JumpIfFalseOrPop(_, _)
            | Instruction::JumpIfTrueOrPop(_, _)
            | Instruction::Iterate(_)
            | Instruction::FastRecurse
            | Instruction::PopFrame
            | Instruction::Return
    )
}

fn branch_targets(cur_idx: usize, inst: &Instruction) -> Vec<(usize, EdgeKind)> {
    use EdgeKind::*;
    match inst {
        Instruction::Jump(t) => vec![(*t, Uncond)],
        Instruction::FastRecurse => vec![(/*loop-head*/ 0, Uncond)],
        Instruction::Iterate(t) => vec![(cur_idx + 1, Cond(true)), (*t, Cond(false))],
        Instruction::JumpIfFalse(t) | Instruction::JumpIfFalseOrPop(t, _) => {
            vec![(cur_idx + 1, Cond(true)), (*t, Cond(false))]
        }
        Instruction::JumpIfTrueOrPop(t, _) => vec![(*t, Cond(true)), (cur_idx + 1, Cond(false))],
        Instruction::PopFrame => vec![(cur_idx + 1, EdgeKind::FallThrough)],
        Instruction::Return => vec![],
        _ => vec![(cur_idx + 1, FallThrough)],
    }
}

// ...existing code...

#[allow(clippy::needless_range_loop)]
pub fn build_cfg(code: &[Instruction]) -> CFG {
    use EdgeKind::*;

    // 1. Identify block leaders (start of each basic block)
    let mut leaders: BTreeSet<usize> = BTreeSet::new();
    leaders.insert(0);
    for (index, instruction) in code.iter().enumerate() {
        for (target, kind) in branch_targets(index, instruction) {
            if !matches!(kind, FallThrough) && target < code.len() {
                leaders.insert(target);
            }
        }
        if is_block_terminator(instruction) && index + 1 < code.len() {
            leaders.insert(index + 1);
        }
    }

    // 2. Build basic blocks and instruction-to-block mapping
    let leader_vec: Vec<_> = leaders.into_iter().collect();
    let mut blocks = Vec::<BasicBlock>::new();
    let mut instruction_to_basic_block = vec![0; code.len()];
    for (i, &start) in leader_vec.iter().enumerate() {
        let end = if i + 1 < leader_vec.len() {
            leader_vec[i + 1] - 1
        } else {
            code.len() - 1
        };
        // Find macro name for this block, if any (only at block start)
        let cur_macro = match code.get(start) {
            Some(Instruction::MacroName(name)) => Some(name.to_string()),
            _ => None,
        };
        blocks.push(BasicBlock {
            id: i,
            start,
            end,
            successor: Vec::new(),
            predecessor: Vec::new(),
            current_macro: cur_macro,
            type_constraints: std::collections::BTreeMap::new(),
        });
        for idx in start..=end {
            instruction_to_basic_block[idx] = i;
        }
    }

    // 3. Collect macro entry points (MacroName at block start)
    // Instead of a map, collect a Vec<(block_idx, macro_name)>
    let mut macro_entries = Vec::new();
    for (i, block) in blocks.iter().enumerate() {
        if let Some(Instruction::MacroName(name)) = code.get(block.start) {
            macro_entries.push((i, name.to_string()));
        }
    }

    // 4. For each BuildMacro, find the nearest previous MacroName block
    let mut macro_succ_edges = Vec::new();
    for (block_id, block) in blocks.iter().enumerate() {
        for idx in block.start..=block.end {
            if let Instruction::BuildMacro(ref macro_name, _target_idx, _) = code[idx] {
                // Find the nearest previous block with MacroName(macro_name)
                if let Some(&(macro_block_id, _)) = macro_entries
                    .iter()
                    .rev()
                    .find(|&&(bidx, ref n)| bidx < block_id && n == macro_name)
                {
                    macro_succ_edges.push((block_id, macro_block_id));
                }
            }
        }
    }

    // 5. Collect type narrowing updates (for type inference)
    let mut type_narrow_updates: Vec<(usize, String, crate::types::builtin::Type)> = Vec::new();

    // 6. Collect control-flow edges (successors and their kinds)
    let mut tmp_succ: Vec<Vec<(BlockId, EdgeKind)>> = vec![Vec::new(); blocks.len()];
    let mut tmp_preds: Vec<Vec<BlockId>> = vec![Vec::new(); blocks.len()];
    for (block_id, block) in blocks.iter().enumerate() {
        let last = block.end;
        for (target, kind) in branch_targets(last, &code[last]) {
            // Type narrowing for string test
            if let Instruction::JumpIfFalse(jump_target) = &code[last] {
                if last > 1 {
                    // look back to see if the previous instruction was a string test
                    if let Instruction::PerformTest(test_name, Some(1), 0) = &code[last - 1] {
                        // only implements string test for now. TODO: add more judgements
                        if *test_name == "string" {
                            // look back to see if a variable was under string test
                            if let Instruction::Lookup(var_name, _) = &code[last - 2] {
                                // Find all blocks whose instructions are in (last+1)..*jump_target*
                                let start_idx = last + 1;
                                let end_idx = *jump_target;
                                for (bidx, b) in blocks.iter().enumerate() {
                                    if b.start >= start_idx && b.end < end_idx {
                                        type_narrow_updates.push((
                                            bidx,
                                            var_name.to_string(),
                                            crate::types::builtin::Type::String,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if target < code.len() {
                let succ_block = instruction_to_basic_block[target];
                tmp_succ[block_id].push((succ_block, kind));
                tmp_preds[succ_block].push(block_id);
            }
        }
    }

    // 7. Assign successors to blocks (including macro call edges)
    for (block_id, succs) in tmp_succ.iter_mut().enumerate() {
        // Add macro call successors
        for &(from, to) in &macro_succ_edges {
            if from == block_id {
                succs.push((to, EdgeKind::Uncond));
            }
        }
        blocks[block_id].successor = succs.clone();
    }

    // 8. Build predecessors from successors (dedup)
    let mut tmp_preds: Vec<Vec<BlockId>> = vec![Vec::new(); blocks.len()];
    for (from, block) in blocks.iter().enumerate() {
        for &(to, _) in &block.successor {
            tmp_preds[to].push(from);
        }
    }
    for (i, preds) in tmp_preds.into_iter().enumerate() {
        let mut all_preds = preds;
        all_preds.sort_unstable();
        all_preds.dedup();
        blocks[i].predecessor = all_preds;
    }

    // 9. Apply type narrowing updates
    for (block_id, var_name, ty) in type_narrow_updates {
        blocks[block_id].type_constraints.insert(var_name, ty);
    }

    CFG {
        blocks,
        instruction_to_basic_block,
        entry: 0,
    }
}
