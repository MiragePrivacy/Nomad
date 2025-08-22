use crate::ArithmeticOp;
use std::{
    collections::HashMap,
    fmt::{Display, Formatter, Result as FmtResult},
    ops::Deref,
};

/// A map that tracks transformations applied to register states
pub struct TransformationMap {
    transformations: Vec<TransformationEntry>,
}

pub struct TransformationEntry {
    input_state: [u32; 8],
    transformation: Transformation,
    output_state: [u32; 8],
}

impl Default for TransformationMap {
    fn default() -> Self {
        Self::new()
    }
}

impl TransformationMap {
    pub fn new() -> Self {
        Self {
            transformations: Vec::new(),
        }
    }

    pub fn add_transformation(
        &mut self,
        input_state: [u32; 8],
        transformation: Transformation,
        output_state: [u32; 8],
    ) {
        self.transformations.push(TransformationEntry {
            input_state,
            transformation,
            output_state,
        });
    }
}

impl Deref for TransformationMap {
    type Target = [TransformationEntry];

    fn deref(&self) -> &Self::Target {
        &self.transformations
    }
}

impl TransformationEntry {
    pub fn input_state(&self) -> &[u32; 8] {
        &self.input_state
    }

    pub fn transformation(&self) -> &Transformation {
        &self.transformation
    }

    pub fn output_state(&self) -> &[u32; 8] {
        &self.output_state
    }
}

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
        }
    }
}

impl Display for TransformationMap {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        writeln!(f, "graph TD")?;
        writeln!(
            f,
            "    classDef arithmetic fill:#e1f5fe,stroke:#01579b,color:#01579b"
        )?;
        writeln!(
            f,
            "    classDef memory fill:#f3e5f5,stroke:#4a148c,color:#4a148c"
        )?;
        writeln!(
            f,
            "    classDef jump fill:#fff3e0,stroke:#e65100,color:#e65100"
        )?;
        writeln!(
            f,
            "    classDef encryption fill:#e8f5e8,stroke:#1b5e20,color:#1b5e20"
        )?;
        writeln!(
            f,
            "    classDef shuffle fill:#fce4ec,stroke:#880e4f,color:#880e4f"
        )?;
        writeln!(
            f,
            "    classDef state fill:#f5f5f5,stroke:#424242,color:#424242"
        )?;
        writeln!(f)?;

        // Track unique states to avoid duplication
        let mut state_counter = 0;
        let mut state_map = HashMap::new();

        // First pass: collect all unique states
        for entry in self.transformations.iter() {
            let input_hash = hash_state(entry.input_state);
            let output_hash = hash_state(*entry.output_state());

            if let std::collections::hash_map::Entry::Vacant(e) = state_map.entry(input_hash) {
                e.insert(state_counter);
                writeln!(
                    f,
                    "    S{}[\"State {}<br/>{}\"]:::state",
                    state_counter,
                    state_counter,
                    format_state(entry.input_state)
                )?;
                state_counter += 1;
            }

            if let std::collections::hash_map::Entry::Vacant(e) = state_map.entry(output_hash) {
                e.insert(state_counter);
                writeln!(
                    f,
                    "    S{}[\"State {}<br/>{}\"]:::state",
                    state_counter,
                    state_counter,
                    format_state(*entry.output_state())
                )?;
                state_counter += 1;
            }
        }

        writeln!(f)?;

        // Second pass: create transformation nodes and connections
        for (i, entry) in self.transformations.iter().enumerate() {
            let input_hash = hash_state(entry.input_state);
            let output_hash = hash_state(*entry.output_state());
            let input_id = state_map[&input_hash];
            let output_id = state_map[&output_hash];

            // Create transformation node with execution order
            let execution_order = i + 1; // 1-based ordering for better readability
            let (transform_label, class) = match entry.transformation() {
                Transformation::ArithmeticChain {
                    operations,
                    registers,
                } => {
                    let ops_str = operations
                        .iter()
                        .map(|op| match op {
                            ArithmeticOp::Add => "ADD",
                            ArithmeticOp::Sub => "SUB",
                            ArithmeticOp::Xor => "XOR",
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let regs_str = registers
                        .iter()
                        .map(|r| format!("R{r}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    (
                        format!(
                            "#{execution_order} Arithmetic<br/>[{ops_str}]<br/>Regs: {regs_str}"
                        ),
                        "arithmetic",
                    )
                }
                Transformation::MemoryScramble { addresses, pattern } => (
                    format!(
                        "#{execution_order} Memory Scramble<br/>{} addrs<br/>Pattern: 0x{:08X}",
                        addresses.len(),
                        pattern
                    ),
                    "memory",
                ),
                Transformation::ConditionalJump {
                    condition_regs,
                    jump_targets,
                } => (
                    format!(
                        "#{execution_order} Conditional Jump<br/>R{} vs R{}<br/>{} targets",
                        condition_regs.0,
                        condition_regs.1,
                        jump_targets.len()
                    ),
                    "jump",
                ),
                Transformation::EncryptionRound { key_addr, rounds } => (
                    format!(
                        "#{execution_order} Encryption<br/>{rounds} rounds<br/>Key@0x{key_addr:08X}"
                    ),
                    "encryption",
                ),
                Transformation::RegisterShuffle { mapping } => {
                    let shuffle_str = mapping
                        .iter()
                        .map(|(src, dst)| format!("R{src}→R{dst}"))
                        .collect::<Vec<_>>()
                        .join("<br/>");
                    (
                        format!("#{execution_order} Register Shuffle<br/>{shuffle_str}"),
                        "shuffle",
                    )
                }
            };

            writeln!(f, "    T{i}[\"{transform_label}\"]:::{class}")?;
            writeln!(f, "    S{input_id} --> T{i}")?;
            writeln!(f, "    T{i} --> S{output_id}")?;
        }

        // Add execution sequence arrows between transformations
        writeln!(f)?;
        writeln!(f, "    %% Execution sequence")?;
        for i in 0..self.transformations.len().saturating_sub(1) {
            writeln!(f, "    T{i} -.-> T{}", i + 1)?;
        }

        // Add final output node if we have transformations
        if !self.transformations.is_empty() {
            writeln!(f)?;
            writeln!(f, "    FINAL[\"Final Output<br/>256-bit result\"]:::state")?;

            // Connect the last transformation's output to final output
            if let Some(last_entry) = self.transformations.last() {
                let output_hash = hash_state(*last_entry.output_state());
                let output_id = state_map[&output_hash];
                writeln!(f, "    S{output_id} --> FINAL")?;
            }
        }

        Ok(())
    }
}

/// Create a simple hash of a register state for deduplication
fn hash_state(state: [u32; 8]) -> u64 {
    let mut hash = 0u64;
    for (i, &val) in state.iter().enumerate() {
        hash = hash
            .wrapping_mul(31)
            .wrapping_add(val as u64)
            .wrapping_add(i as u64);
    }
    hash
}

/// Format a register state for display
fn format_state(state: [u32; 8]) -> String {
    format!(
        "R0-7: {:08X} {:08X}<br/>{:08X} {:08X}<br/>{:08X} {:08X}<br/>{:08X} {:08X}",
        state[0], state[1], state[2], state[3], state[4], state[5], state[6], state[7]
    )
}
