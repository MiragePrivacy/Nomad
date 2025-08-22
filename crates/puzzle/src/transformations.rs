use crate::ArithmeticOp;
use std::{
    collections::HashMap,
    fmt::{Display, Formatter, Result as FmtResult},
};

/// Different types of transformations that can be applied to register states
#[derive(Debug, Clone)]
pub enum Transformation {
    /// Chain of arithmetic operations on specified registers
    ArithmeticChain {
        operations: Vec<ArithmeticOp>,
        registers: Vec<u8>,
    },

    /// Complex memory read/write patterns to obfuscate data flow
    MemoryScramble {
        addresses: Vec<u32>,
        pattern: u32, // Bit pattern for read/write operations
    },

    /// Variable conditional jumps creating complex control flow
    ConditionalJump {
        condition_regs: (u8, u8), // Registers to compare
        jump_targets: Vec<u32>,   // Possible jump destinations
    },

    /// XOR-based encryption using rotating keys from memory
    EncryptionRound {
        key_addr: u32, // Memory address containing encryption key
        rounds: u8,    // Number of encryption rounds
    },

    /// Shuffle register values in unpredictable patterns
    RegisterShuffle {
        mapping: HashMap<u8, u8>, // Source register -> destination register
    },

    /// Split data flow into multiple parallel paths
    Split {},

    /// Rejoin multiple parallel paths into fewer outputs
    Rejoin {},
}

impl Transformation {
    /// Estimate the number of instructions this transformation will generate
    pub fn estimated_instruction_count(&self) -> usize {
        match self {
            Transformation::ArithmeticChain {
                operations,
                registers,
            } => {
                // Each operation requires temporary register loads
                operations.len() * (registers.len() + 2)
            }
            Transformation::MemoryScramble { addresses, .. } => {
                // Each address requires load/store pair
                addresses.len() * 4
            }
            Transformation::ConditionalJump { jump_targets, .. } => {
                // Conditional jumps with setup
                jump_targets.len() * 3 + 5
            }
            Transformation::EncryptionRound { rounds, .. } => {
                // Key load + encryption operations
                (*rounds as usize) * 12 + 4
            }
            Transformation::RegisterShuffle { mapping } => {
                // Register moves with temporary storage
                mapping.len() * 3
            }
            Transformation::Split {} | Transformation::Rejoin {} => 0,
        }
    }

    /// Check if this transformation creates variable control flow
    pub fn has_variable_jumps(&self) -> bool {
        matches!(self, Transformation::ConditionalJump { .. })
    }

    /// Check if this transformation uses memory operations
    pub fn uses_memory(&self) -> bool {
        matches!(
            self,
            Transformation::MemoryScramble { .. } | Transformation::EncryptionRound { .. }
        )
    }

    /// Get the memory addresses used by this transformation
    pub fn memory_addresses(&self) -> Vec<u32> {
        match self {
            Transformation::MemoryScramble { addresses, .. } => addresses.clone(),
            Transformation::EncryptionRound { key_addr, .. } => vec![*key_addr],
            _ => Vec::new(),
        }
    }

    /// Get the registers affected by this transformation
    pub fn affected_registers(&self) -> Vec<u8> {
        match self {
            Transformation::ArithmeticChain { registers, .. } => registers.clone(),
            Transformation::ConditionalJump { condition_regs, .. } => {
                vec![condition_regs.0, condition_regs.1]
            }
            Transformation::EncryptionRound { .. } => (0..8).collect(),
            Transformation::RegisterShuffle { mapping } => mapping.keys().copied().collect(),
            Transformation::MemoryScramble { .. } => (0..8).collect(),
            Transformation::Split { .. } => (0..8).collect(), // Affects all registers during split
            Transformation::Rejoin { .. } => (0..8).collect(), // Affects all registers during rejoin
        }
    }
}

impl Display for Transformation {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Transformation::ArithmeticChain {
                operations,
                registers,
            } => {
                let ops = operations
                    .iter()
                    .map(|op| match op {
                        ArithmeticOp::Add => "ADD",
                        ArithmeticOp::Sub => "SUB",
                        ArithmeticOp::Xor => "XOR",
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let regs = registers
                    .iter()
                    .map(|r| format!("R{r}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "Arithmetic Chain [{ops}] on registers [{regs}]")
            }
            Transformation::MemoryScramble { addresses, pattern } => {
                write!(
                    f,
                    "Memory Scramble ({} addresses, pattern 0x{:08X})",
                    addresses.len(),
                    pattern
                )
            }
            Transformation::ConditionalJump {
                condition_regs,
                jump_targets,
            } => {
                write!(
                    f,
                    "Conditional Jump (R{} vs R{}, {} targets)",
                    condition_regs.0,
                    condition_regs.1,
                    jump_targets.len()
                )
            }
            Transformation::EncryptionRound { key_addr, rounds } => {
                write!(f, "Encryption ({rounds} rounds, key@0x{key_addr:08X})")
            }
            Transformation::RegisterShuffle { mapping } => {
                write!(f, "Register Shuffle ({} mappings)", mapping.len())
            }
            Transformation::Split {} => {
                write!(f, "Split")
            }
            Transformation::Rejoin {} => {
                write!(f, "Rejoin")
            }
        }
    }
}
