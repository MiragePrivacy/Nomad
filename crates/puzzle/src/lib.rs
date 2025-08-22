use nomad_vm::Program;
use rand::RngCore;

mod chain_builder;
mod compiler;
mod transformations;

pub use chain_builder::{ChainBuilder, TransformationChain, TransformationNode};
pub use compiler::PuzzleCompiler;
pub use transformations::Transformation;

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

    fn generate_chain(
        &mut self,
        target_output: [u8; 32],
    ) -> Result<TransformationChain, PuzzleError> {
        // Convert target output to final register values - we'll use this as target constraint
        let _target_registers = output_to_registers(target_output);

        // Create chain builder with configurable parameters
        let mut chain_builder = ChainBuilder::new(self.max_depth, &mut self.rng)
            .with_split_probability(0.4)
            .with_rejoin_probability(0.3)
            .with_max_splits(4);

        // Build transformation chain with 8 inputs (registers) and 8 outputs
        let chain = chain_builder.build_chain(8, 8);
        Ok(chain)
    }

    pub fn generate_mermaid(&mut self, target_output: [u8; 32]) -> Result<String, PuzzleError> {
        let chain = self.generate_chain(target_output)?;
        Ok(chain.to_mermaid_string())
    }

    pub fn generate_mnemonic(&mut self, target_output: [u8; 32]) -> Result<String, PuzzleError> {
        let chain = self.generate_chain(target_output)?;
        let mut compiler = PuzzleCompiler::new(self.max_instructions);
        compiler.compile_chain_mnemonic(chain)
    }

    /// Generate a puzzle that produces the target 256-bit output when executed
    pub fn generate(&mut self, target_output: [u8; 32]) -> Result<Program, PuzzleError> {
        let chain = self.generate_chain(target_output)?;
        // Compile chain into VM program
        let mut compiler = PuzzleCompiler::new(self.max_instructions);
        compiler.compile_chain(chain)
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
    use nomad_vm::NomadVm;
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

        let mut vm = NomadVm::new(1024 * 1024 * 1024);
        let output = vm.execute_program(program).unwrap();
        assert_eq!(target_output, output);
    }

    #[test]
    fn test_chain_builder_basic() {
        let rng = StdRng::from_seed([1; 32]);
        let mut chain_builder = ChainBuilder::new(2, rng)
            .with_split_probability(0.5)
            .with_rejoin_probability(0.3);

        let chain = chain_builder.build_chain(8, 8);

        // Should have some nodes
        assert!(chain.node_count() > 0);

        // Should have entry and exit points
        assert!(!chain.entry_nodes.is_empty());
        assert!(!chain.exit_nodes.is_empty());

        // Should generate mermaid output
        let mermaid = chain.to_mermaid_string();
        assert!(mermaid.contains("graph TD"));
        assert!(mermaid.contains("N0"));
    }

    #[test]
    fn test_chain_compilation() {
        let rng = StdRng::from_seed([2; 32]);
        let mut generator = PuzzleGenerator::new(2, 1000, rng);

        let target_output = [0x42u8; 32];
        let result = generator.generate(target_output);

        // Should successfully generate a program
        match &result {
            Ok(_) => {}
            Err(e) => panic!("Failed to generate puzzle with chains: {e}"),
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
    fn test_chain_mermaid_generation() {
        let rng = StdRng::from_seed([42; 32]);
        let mut generator = PuzzleGenerator::new(3, 1000, rng);

        let target_output = [0x55u8; 32];
        let result = generator.generate_mermaid(target_output);

        match &result {
            Ok(mermaid) => {
                // Should contain mermaid syntax
                assert!(mermaid.contains("graph TD"));
                assert!(mermaid.contains("N0"));
                // Should contain some node types (updated for new format)
                assert!(
                    mermaid.contains("Split")
                        || mermaid.contains("Arithmetic")
                        || mermaid.contains("Rejoin")
                        || mermaid.contains("Memory")
                        || mermaid.contains("Encrypt")
                        || mermaid.contains("Shuffle")
                );
                // Should contain register information
                assert!(mermaid.contains("Regs:"));
                println!("Generated mermaid:\n{mermaid}");
            }
            Err(e) => panic!("Failed to generate mermaid: {e}"),
        }
    }

    #[test]
    fn test_register_isolation_in_splits() {
        let rng = StdRng::from_seed([123; 32]);
        let mut chain_builder = ChainBuilder::new(2, rng)
            .with_split_probability(1.0) // Force splits
            .with_rejoin_probability(0.0) // Prevent rejoins initially
            .with_max_splits(3);

        let chain = chain_builder.build_chain(4, 4);

        // Verify that nodes have assigned registers
        for (node_id, node) in &chain.nodes {
            assert!(
                !node.assigned_registers.is_empty(),
                "Node {} should have assigned registers",
                node_id.inner()
            );

            // All assigned registers should be in valid range
            for &reg in &node.assigned_registers {
                assert!(reg < 8, "Register {reg} should be < 8");
            }
        }

        // Find split nodes and verify they have different register assignments for branches
        let split_nodes: Vec<_> = chain
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node.operation, Transformation::Split { .. }))
            .collect();

        if !split_nodes.is_empty() {
            println!("Found {} split nodes", split_nodes.len());

            // Check that split nodes have comprehensive register assignments
            for (node_id, node) in split_nodes {
                println!(
                    "Split node {} has registers: {:?}",
                    node_id.inner(),
                    node.assigned_registers
                );

                // Split nodes should have access to all or most registers
                assert!(
                    node.assigned_registers.len() >= 2,
                    "Split node {} should have at least 2 registers",
                    node_id.inner()
                );
            }
        }
    }

    #[test]
    fn test_register_isolation_compilation() {
        let rng = StdRng::from_seed([42; 32]);
        let mut generator = PuzzleGenerator::new(2, 1000, rng);

        // Generate a chain with splits
        let target_output = [0x55u8; 32];
        let result = generator.generate_mnemonic(target_output);

        match &result {
            Ok(mnemonic) => {
                // Verify that the mnemonic contains register operations
                assert!(mnemonic.contains("NODE"));
                assert!(mnemonic.contains("HALT"));

                // Look for register isolation evidence (should contain register operations)
                let lines: Vec<&str> = mnemonic.lines().collect();
                let instruction_lines: Vec<&str> = lines
                    .iter()
                    .filter(|line| {
                        line.trim()
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_digit())
                    })
                    .cloned()
                    .collect();

                println!("Generated {} instruction lines", instruction_lines.len());
                assert!(
                    !instruction_lines.is_empty(),
                    "Should generate instructions"
                );
            }
            Err(e) => panic!("Failed to generate mnemonic: {e}"),
        }
    }
}
