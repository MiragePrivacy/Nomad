use crate::{
    chain_builder::TransformationChain, transformations::Transformation, ArithmeticOp, PuzzleError,
};
use nomad_vm::{Instruction, Program};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::collections::HashMap;

/// Compiles transformation chains into executable VM programs
pub struct PuzzleCompiler {
    max_instructions: usize,
    rng: StdRng,
}

impl PuzzleCompiler {
    pub fn new(max_instructions: usize) -> Self {
        Self {
            max_instructions,
            rng: StdRng::from_os_rng(),
        }
    }

    /// Compile a transformation chain into a VM program
    pub fn compile_chain(&mut self, chain: TransformationChain) -> Result<Program, PuzzleError> {
        let mut instructions = Vec::new();

        // Initialize memory with encryption keys and random data
        self.initialize_memory(&mut instructions)?;

        // Compile nodes in topological order
        let mut ordered_nodes = self.topological_sort(&chain);
        ordered_nodes.reverse();
        for node_id in ordered_nodes {
            if let Some(node) = chain.nodes.get(&node_id) {
                self.compile_chain_node(&mut instructions, node)?;
                // self.add_obfuscation(&mut instructions)?;
            }
        }

        // Add halt instruction
        instructions.push(Instruction::Halt());

        // Check instruction count
        if instructions.len() > self.max_instructions {
            return Err(PuzzleError::TooManyInstructions);
        }

        Ok(Program::from_raw(instructions))
    }

    /// Generate mnemonic output for transformation chain
    pub fn compile_chain_mnemonic(
        &mut self,
        chain: TransformationChain,
    ) -> Result<String, PuzzleError> {
        let mut output = String::new();
        let mut instruction_counter = 0;

        output.push_str("; ========================================\n");
        output.push_str("; TRANSFORMATION CHAIN DEBUG OUTPUT\n");
        output.push_str(&format!(
            "; Generated with {} nodes, {} connections\n",
            chain.node_count(),
            chain.connection_count()
        ));
        output.push_str("; ========================================\n\n");

        // Initialize memory
        output.push_str("; Memory Initialization\n");
        let mut instructions = Vec::new();
        self.initialize_memory(&mut instructions)?;

        for (i, instr) in instructions.iter().enumerate() {
            output.push_str(&format!("{:4}: {}\n", instruction_counter + i, instr));
        }
        instruction_counter += instructions.len();
        output.push('\n');

        // Compile nodes in order
        let ordered_nodes = self.topological_sort(&chain);
        for (idx, node_id) in ordered_nodes.iter().enumerate() {
            if let Some(node) = chain.nodes.get(node_id) {
                output.push_str("; ----------------------------------------\n");
                output.push_str(&format!(
                    "; NODE {}: {:?}\n",
                    node_id.inner(),
                    node.operation
                ));
                output.push_str("; ----------------------------------------\n");

                let mut node_instructions = Vec::new();
                self.compile_chain_node(&mut node_instructions, node)?;

                for (i, instr) in node_instructions.iter().enumerate() {
                    output.push_str(&format!("{:4}: {}\n", instruction_counter + i, instr));
                }
                instruction_counter += node_instructions.len();

                // Add obfuscation
                if idx < ordered_nodes.len() - 1 {
                    output.push_str("\n; Obfuscation\n");
                    let mut obf_instructions = Vec::new();
                    self.add_obfuscation(&mut obf_instructions)?;
                    for (i, instr) in obf_instructions.iter().enumerate() {
                        output.push_str(&format!("{:4}: {}\n", instruction_counter + i, instr));
                    }
                    instruction_counter += obf_instructions.len();
                }
                output.push('\n');
            }
        }

        // Halt instruction
        output.push_str(&format!("{instruction_counter:4}: HALT\n"));

        Ok(output)
    }

    fn initialize_memory(
        &mut self,
        instructions: &mut Vec<Instruction>,
    ) -> Result<(), PuzzleError> {
        // Initialize random encryption keys in memory
        for i in 0..16 {
            let addr = (i * 16) as u32; // Spread keys across memory
            let key = self.rng.random();

            // Store key in memory using a temporary register
            instructions.push(Instruction::Set(7, key)); // Use R7 as temporary
            instructions.push(Instruction::Store(7, addr));
        }

        Ok(())
    }

    fn add_obfuscation(&mut self, instructions: &mut Vec<Instruction>) -> Result<(), PuzzleError> {
        // Add 2-4 dead instructions for obfuscation
        let count = self.rng.random_range(2..=4);

        for _ in 0..count {
            match self.rng.random_range(0..3) {
                0 => {
                    // Dead arithmetic
                    let reg = self.rng.random_range(0..8) as u8;
                    let val = self.rng.random::<u32>();
                    instructions.push(Instruction::Set(reg, val));
                    instructions.push(Instruction::Add(reg, reg, reg)); // Double the value
                    instructions.push(Instruction::Sub(reg, reg, reg)); // Zero it out
                }
                1 => {
                    // Dead memory operation
                    let addr = self.rng.random_range(1000..2000);
                    instructions.push(Instruction::Set(6, 0));
                    instructions.push(Instruction::Store(6, addr));
                    instructions.push(Instruction::Load(6, addr));
                }
                _ => {
                    // Dead XOR (always results in zero)
                    let reg = self.rng.random_range(0..8) as u8;
                    instructions.push(Instruction::Xor(reg, reg, reg));
                }
            }
        }

        Ok(())
    }

    /// Simple topological sort for chain nodes
    fn topological_sort(&self, chain: &TransformationChain) -> Vec<crate::chain_builder::NodeId> {
        use std::collections::VecDeque;

        let mut in_degree = HashMap::new();
        let mut adj_list = HashMap::new();

        // Initialize in-degree count and adjacency list
        for node_id in chain.nodes.keys() {
            in_degree.insert(*node_id, 0);
            adj_list.insert(*node_id, Vec::new());
        }

        // Build adjacency list and calculate in-degrees
        for conn in &chain.connections {
            adj_list
                .get_mut(&conn.from_node)
                .unwrap()
                .push(conn.to_node);
            *in_degree.get_mut(&conn.to_node).unwrap() += 1;
        }

        // Start with nodes that have no incoming edges
        let mut queue = VecDeque::new();
        for (&node_id, &degree) in &in_degree {
            if degree == 0 {
                queue.push_back(node_id);
            }
        }

        let mut result = Vec::new();
        while let Some(node_id) = queue.pop_front() {
            result.push(node_id);

            // Reduce in-degree for all neighbors
            if let Some(neighbors) = adj_list.get(&node_id) {
                for &neighbor in neighbors {
                    *in_degree.get_mut(&neighbor).unwrap() -= 1;
                    if in_degree[&neighbor] == 0 {
                        queue.push_back(neighbor);
                    }
                }
            }
        }

        result
    }

    /// Compile a single chain node to instructions
    fn compile_chain_node(
        &mut self,
        instructions: &mut Vec<Instruction>,
        node: &crate::chain_builder::TransformationNode,
    ) -> Result<(), PuzzleError> {
        self.compile_transformation(instructions, &node.operation, &node.assigned_registers)
    }

    /// Compile a transformation into instructions
    fn compile_transformation(
        &mut self,
        instructions: &mut Vec<Instruction>,
        transformation: &Transformation,
        assigned_registers: &[u8],
    ) -> Result<(), PuzzleError> {
        match transformation {
            Transformation::ArithmeticChain {
                operations,
                registers,
            } => self.compile_arithmetic_chain(instructions, operations, registers),
            Transformation::MemoryScramble { addresses, pattern } => {
                self.compile_memory_scramble(instructions, addresses, *pattern, assigned_registers)
            }
            Transformation::ConditionalJump {
                condition_regs,
                jump_targets,
            } => self.compile_conditional_jump(instructions, *condition_regs, jump_targets),
            Transformation::EncryptionRound { key_addr, rounds } => {
                self.compile_encryption_round(instructions, *key_addr, *rounds, assigned_registers)
            }
            Transformation::RegisterShuffle { mapping } => {
                self.compile_register_shuffle(instructions, mapping)
            }
            Transformation::Split { .. } => {
                // Split is metadata-only for register routing, generates no instructions
                Ok(())
            }
            Transformation::Rejoin { .. } => {
                // Rejoin is metadata-only for register routing, generates no instructions
                Ok(())
            }
        }
    }

    fn compile_arithmetic_chain(
        &mut self,
        instructions: &mut Vec<Instruction>,
        operations: &[ArithmeticOp],
        registers: &[u8],
    ) -> Result<(), PuzzleError> {
        // Use first register as accumulator
        if let Some(&acc_reg) = registers.first() {
            for (i, operation) in operations.iter().enumerate() {
                let src_reg = registers.get(i % registers.len()).copied().unwrap_or(0);
                let dst_reg = registers
                    .get((i + 1) % registers.len())
                    .copied()
                    .unwrap_or(acc_reg);

                match operation {
                    ArithmeticOp::Add => {
                        instructions.push(Instruction::Add(acc_reg, src_reg, dst_reg));
                    }
                    ArithmeticOp::Sub => {
                        instructions.push(Instruction::Sub(acc_reg, src_reg, dst_reg));
                    }
                    ArithmeticOp::Xor => {
                        instructions.push(Instruction::Xor(acc_reg, src_reg, dst_reg));
                    }
                }
            }
        }

        Ok(())
    }

    fn compile_memory_scramble(
        &mut self,
        instructions: &mut Vec<Instruction>,
        addresses: &[u32],
        pattern: u32,
        assigned_registers: &[u8],
    ) -> Result<(), PuzzleError> {
        let temp_reg = self.find_temp_register(assigned_registers);

        for (i, &addr) in addresses.iter().enumerate() {
            let reg = if i < assigned_registers.len() {
                assigned_registers[i]
            } else {
                assigned_registers[i % assigned_registers.len()]
            };

            // Apply pattern-based transformation
            let transformed_pattern = pattern.wrapping_mul(i as u32 + 1);
            instructions.push(Instruction::Set(temp_reg, transformed_pattern));

            // Load from memory, transform with pattern, store back
            instructions.push(Instruction::Load(reg, addr));
            instructions.push(Instruction::Xor(reg, temp_reg, reg));
            instructions.push(Instruction::Store(reg, addr));
        }

        Ok(())
    }

    fn compile_conditional_jump(
        &mut self,
        instructions: &mut Vec<Instruction>,
        condition_regs: (u8, u8),
        jump_targets: &[u32],
    ) -> Result<(), PuzzleError> {
        // Simple conditional jump implementation
        let (reg1, reg2) = condition_regs;

        // Compare registers and jump based on result
        for (i, &target) in jump_targets.iter().enumerate() {
            if i == 0 {
                instructions.push(Instruction::JmpNe(reg1, reg2, target));
            }
        }

        Ok(())
    }

    fn compile_encryption_round(
        &mut self,
        instructions: &mut Vec<Instruction>,
        key_addr: u32,
        rounds: u8,
        assigned_registers: &[u8],
    ) -> Result<(), PuzzleError> {
        let key_reg = self.find_temp_register(assigned_registers);

        // Load key from memory
        instructions.push(Instruction::Load(key_reg, key_addr));

        // Apply encryption rounds only to assigned registers
        for round in 0..rounds {
            for &reg in assigned_registers {
                instructions.push(Instruction::Xor(reg, key_reg, reg));
                // Rotate key for next round
                if round < rounds - 1 {
                    instructions.push(Instruction::Add(key_reg, 1, key_reg));
                }
            }
        }

        Ok(())
    }

    fn compile_register_shuffle(
        &mut self,
        instructions: &mut Vec<Instruction>,
        mapping: &HashMap<u8, u8>,
    ) -> Result<(), PuzzleError> {
        let temp_reg = 7u8;

        // Perform register shuffling using temporary storage
        for (&src, &dst) in mapping.iter() {
            if src != dst {
                instructions.push(Instruction::Add(temp_reg, src, 0)); // Copy src to temp
                instructions.push(Instruction::Add(dst, temp_reg, 0)); // Copy temp to dst
            }
        }

        Ok(())
    }

    /// Find a temporary register not in the assigned set
    fn find_temp_register(&self, assigned_registers: &[u8]) -> u8 {
        for reg in 0..8u8 {
            if !assigned_registers.contains(&reg) {
                return reg;
            }
        }
        // If all registers are assigned, use R7 as fallback
        7
    }
}
