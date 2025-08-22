use nomad_vm::Program;
use rand::{Rng, RngCore};
use std::collections::HashMap;

mod compiler;
mod transformations;

pub use compiler::PuzzleCompiler;
pub use transformations::{Transformation, TransformationMap};

/// A puzzle generator that creates VM programs with complex transformations
/// designed to prevent static analysis while producing a deterministic output.
pub struct PuzzleGenerator<R: RngCore> {
    max_depth: usize,
    max_instructions: usize,
    rng: R,
}

impl<R: RngCore> PuzzleGenerator<R> {
    pub fn new(max_depth: usize, max_instructions: usize, rng: R) -> Self {
        Self {
            max_depth,
            max_instructions,
            rng,
        }
    }

    fn generate_map(&mut self, target_output: [u8; 32]) -> Result<TransformationMap, PuzzleError> {
        let mut transformation_map = TransformationMap::new();
        // Convert target output to initial register values
        let target_registers = output_to_registers(target_output);
        // Build recursive transformation chain
        self.build_transformations(&mut transformation_map, target_registers, 0)?;
        Ok(transformation_map)
    }

    pub fn generate_mermaid(&mut self, target_output: [u8; 32]) -> Result<String, PuzzleError> {
        let map = self.generate_map(target_output)?;
        Ok(map.to_string())
    }

    pub fn generate_mnemonic(&mut self, target_output: [u8; 32]) -> Result<String, PuzzleError> {
        let transformation_map = self.generate_map(target_output)?;
        let mut compiler = PuzzleCompiler::new(self.max_instructions);
        compiler.compile_mnemonic(transformation_map)
    }

    /// Generate a puzzle that produces the target 256-bit output when executed
    pub fn generate(&mut self, target_output: [u8; 32]) -> Result<Program, PuzzleError> {
        let transformation_map = self.generate_map(target_output)?;
        // Compile transformations into VM program
        let mut compiler = PuzzleCompiler::new(self.max_instructions);
        compiler.compile(transformation_map)
    }

    fn build_transformations(
        &mut self,
        map: &mut TransformationMap,
        target_state: [u32; 8],
        depth: usize,
    ) -> Result<(), PuzzleError> {
        if depth >= self.max_depth {
            return Ok(());
        }

        // Generate random transformations for this depth level
        let num_transforms = self.rng.random_range(2..=5);

        for _ in 0..num_transforms {
            let transform = self.generate_random_transformation();
            let input_state = self.generate_random_state();

            map.add_transformation(input_state, transform, target_state);

            // Recursively build transformations for the input state
            self.build_transformations(map, input_state, depth + 1)?;
        }

        Ok(())
    }

    fn generate_random_transformation(&mut self) -> Transformation {
        match self.rng.random_range(0..5) {
            0 => Transformation::ArithmeticChain {
                operations: self.generate_arithmetic_ops(),
                registers: self.generate_register_set(),
            },
            1 => Transformation::MemoryScramble {
                addresses: self.generate_memory_addresses(),
                pattern: self.rng.random(),
            },
            2 => Transformation::ConditionalJump {
                condition_regs: (self.rng.random_range(0..8), self.rng.random_range(0..8)),
                jump_targets: self.generate_jump_targets(),
            },
            3 => Transformation::EncryptionRound {
                key_addr: self.rng.random_range(0..1024 * 1024) * 4, // Align to word boundaries
                rounds: self.rng.random_range(1..=4),
            },
            4 => Transformation::RegisterShuffle {
                mapping: self.generate_shuffle_mapping(),
            },
            _ => unreachable!(),
        }
    }

    fn generate_random_state(&mut self) -> [u32; 8] {
        let mut state = [0u32; 8];
        for i in &mut state {
            *i = self.rng.random();
        }
        state
    }

    fn generate_arithmetic_ops(&mut self) -> Vec<ArithmeticOp> {
        let count = self.rng.random_range(2..=6);
        (0..count)
            .map(|_| match self.rng.random_range(0..3) {
                0 => ArithmeticOp::Add,
                1 => ArithmeticOp::Sub,
                2 => ArithmeticOp::Xor,
                _ => unreachable!(),
            })
            .collect()
    }

    fn generate_register_set(&mut self) -> Vec<u8> {
        let count = self.rng.random_range(3..=8);
        let mut registers: Vec<u8> = (0..8).collect();
        registers.truncate(count);
        for i in 0..count {
            let j = self.rng.random_range(i..count);
            registers.swap(i, j);
        }
        registers
    }

    fn generate_memory_addresses(&mut self) -> Vec<u32> {
        let count = self.rng.random_range(4..=16);
        (0..count)
            .map(|_| {
                // Generate aligned addresses within 1GB space
                self.rng.random_range(0..256 * 1024 * 1024) * 4
            })
            .collect()
    }

    fn generate_jump_targets(&mut self) -> Vec<u32> {
        let count = self.rng.random_range(2..=4);
        (0..count).map(|_| self.rng.random_range(1..=100)).collect()
    }

    fn generate_shuffle_mapping(&mut self) -> HashMap<u8, u8> {
        let mut mapping = HashMap::new();
        let mut targets: Vec<u8> = (0..8).collect();

        // Shuffle target registers
        for i in 0..8 {
            let j = self.rng.random_range(i..8);
            targets.swap(i, j);
        }

        for (src, &dst) in (0..8u8).zip(targets.iter()) {
            mapping.insert(src, dst);
        }
        mapping
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ArithmeticOp {
    Add,
    Sub,
    Xor,
}

#[derive(Debug, thiserror::Error)]
pub enum PuzzleError {
    #[error("Maximum instruction count exceeded")]
    TooManyInstructions,
    #[error("Invalid transformation chain")]
    InvalidTransformation,
    #[error("Compilation failed: {0}")]
    CompilationError(String),
}

/// Convert 256-bit output to 8 32-bit register values
fn output_to_registers(output: [u8; 32]) -> [u32; 8] {
    let mut registers = [0u32; 8];
    for (i, register) in registers.iter_mut().enumerate() {
        let offset = i * 4;
        *register = u32::from_be_bytes([
            output[offset],
            output[offset + 1],
            output[offset + 2],
            output[offset + 3],
        ]);
    }
    registers
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn test_output_to_registers() {
        let output = [0u8; 32];
        let registers = output_to_registers(output);
        assert_eq!(registers, [0u32; 8]);

        let mut output = [0u8; 32];
        output[0] = 0x12;
        output[1] = 0x34;
        output[2] = 0x56;
        output[3] = 0x78;
        let registers = output_to_registers(output);
        assert_eq!(registers[0], 0x12345678);
    }

    #[test]
    fn test_puzzle_generator_creation() {
        let rng = StdRng::from_seed([0; 32]);
        let generator = PuzzleGenerator::new(3, 1000, rng);
        assert_eq!(generator.max_depth, 3);
        assert_eq!(generator.max_instructions, 1000);
    }

    #[test]
    fn test_transformation_map() {
        let mut map = TransformationMap::new();
        assert_eq!(map.len(), 0);

        let input_state = [1, 2, 3, 4, 5, 6, 7, 8];
        let output_state = [8, 7, 6, 5, 4, 3, 2, 1];
        let transform = Transformation::ArithmeticChain {
            operations: vec![ArithmeticOp::Add],
            registers: vec![0, 1],
        };

        map.add_transformation(input_state, transform, output_state);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_generate_simple_puzzle() {
        let rng = StdRng::from_seed([42; 32]);
        let mut generator = PuzzleGenerator::new(1, 1000, rng); // Reduced depth, increased instruction limit

        let target_output = [0x42u8; 32];
        let result = generator.generate(target_output);

        // Should successfully generate a program
        match &result {
            Ok(_) => {}
            Err(e) => panic!("Failed to generate puzzle: {e}"),
        }
        let program = result.unwrap();

        // Program should have some instructions
        assert!(!program.is_empty());

        // Should end with HALT
        if let Some(last_instruction) = program.last() {
            assert_eq!(*last_instruction, nomad_vm::Instruction::Halt());
        }
    }

    #[test]
    fn test_transformation_map_mermaid_display() {
        let mut map = TransformationMap::new();

        let state1 = [1, 2, 3, 4, 5, 6, 7, 8];
        let state2 = [8, 7, 6, 5, 4, 3, 2, 1];

        map.add_transformation(
            state1,
            Transformation::ArithmeticChain {
                operations: vec![ArithmeticOp::Add, ArithmeticOp::Xor],
                registers: vec![0, 1, 2],
            },
            state2,
        );

        let mermaid_output = format!("{map}");

        // Check that output contains expected Mermaid syntax
        assert!(mermaid_output.contains("graph TD"));
        assert!(mermaid_output.contains("classDef arithmetic"));
        assert!(mermaid_output.contains("#1 Arithmetic")); // Now includes execution order
        assert!(mermaid_output.contains("ADD, XOR"));
        assert!(mermaid_output.contains("R0, R1, R2"));
        assert!(mermaid_output.contains("-->"));
        assert!(mermaid_output.contains("Final Output"));
        assert!(mermaid_output.contains("%% Execution sequence")); // Check for execution sequence comment
    }
}
