use affair::{DedicatedThread, Executor, Socket, Worker};
use thiserror::Error;
use tracing::{instrument, trace, Span};

pub use crate::instructions::*;
pub use crate::program::*;

mod instructions;
mod program;

/// Fixed memory size available to the VM
pub const MEMORY_SIZE: usize = 1024 * 1024 * 1024;
/// Number of registers available to the VM
pub const REGISTERS: usize = 8;

/// Type alias for the thread worker socket
pub type VmSocket = Socket<<NomadVm as Worker>::Request, <NomadVm as Worker>::Response>;

#[derive(Debug, Error)]
pub enum VmError {
    #[error("Invalid instruction: {0}")]
    InvalidInstruction(u8),
    #[error("Program counter out of bounds: {0}")]
    PcOutOfBounds(usize),
    #[error("Memory address out of bounds: {0}")]
    MemoryOutOfBounds(usize),
    #[error("Invalid register: {0} (must be 0-7)")]
    InvalidRegister(u8),
    #[error("Invalid program format")]
    InvalidProgram,
}

/// A simple VM for executing signal puzzles.
///
/// ## Features
///
/// - 1 GiB memory space
/// - 8x 32-bit registers
/// - Max cycle count of 10k instructions
/// - Registers are concatinated for 256-bit program output
///
/// ## Running as a worker
///
/// A worker can be spawned on a dedicated thread using the helper
/// method [`NomadVm::spawn`] or by using [`affair`] directly.
pub struct NomadVm {
    memory: Vec<u8>,
    registers: [u32; REGISTERS],
    pc: usize,
    max_cycles: usize,
}

impl Worker for NomadVm {
    type Request = (Vec<u8>, Span);
    type Response = Result<[u8; 32], VmError>;

    #[instrument(name = "vm", skip_all, parent = span)]
    fn handle(&mut self, (program, span): Self::Request) -> Self::Response {
        trace!("Received {} byte program", program.len());
        self.execute(program)
    }
}

impl NomadVm {
    /// Create a new VM instance with a given max number of cycles per execution
    pub fn new(max_cycles: usize) -> Self {
        Self {
            memory: vec![0u8; MEMORY_SIZE], // 1 GiB
            registers: [0u32; 8],
            pc: 0,
            max_cycles,
        }
    }

    /// Spawn a new dedicated thread to run the vm worker on
    pub fn spawn(self) -> VmSocket {
        DedicatedThread::spawn(self)
    }

    /// Parse, validate, and execute raw bytecode, returning the result from the concatinated registers
    pub fn execute(&mut self, bytecode: Vec<u8>) -> Result<[u8; 32], VmError> {
        self.execute_program(Program::from_bytes(&bytecode)?)
    }

    /// Executes a program, resets, and returns the result from the concatinated registers.
    fn execute_program(&mut self, program: Program) -> Result<[u8; 32], VmError> {
        let instructions = program.0;

        // Execute instructions
        let mut cycles = 0;
        let mut should_continue = true;
        while should_continue && self.pc < instructions.len() && cycles < self.max_cycles {
            let instruction = &instructions[self.pc];
            should_continue = self.execute_instruction(instruction, instructions.len())?;
            cycles += 1;
        }

        // Compute result from register values
        let mut result = [0u8; 32];
        for (i, &val) in self.registers.iter().enumerate() {
            let bytes = val.to_be_bytes();
            let offset = i * 4;
            result[offset..offset + 3].copy_from_slice(&bytes);
        }

        // Reset the VM state
        self.memory.fill(0);
        self.registers.fill(0);
        self.pc = 0;

        Ok(result)
    }

    /// Execute a single instruction
    fn execute_instruction(
        &mut self,
        instruction: &Instruction,
        instructions_len: usize,
    ) -> Result<bool, VmError> {
        match instruction {
            Instruction::Set(reg, value) => {
                self.registers[*reg as usize] = *value;
                self.pc += 1;
            }
            Instruction::Load(reg, addr) => {
                let addr = *addr as usize;
                if addr + 3 >= self.memory.len() {
                    return Err(VmError::MemoryOutOfBounds(addr));
                }
                let bytes = [
                    self.memory[addr],
                    self.memory[addr + 1],
                    self.memory[addr + 2],
                    self.memory[addr + 3],
                ];
                self.registers[*reg as usize] = u32::from_be_bytes(bytes);
                self.pc += 1;
            }
            Instruction::Store(reg, addr) => {
                let addr = *addr as usize;
                if addr + 3 >= self.memory.len() {
                    return Err(VmError::MemoryOutOfBounds(addr));
                }
                let bytes = self.registers[*reg as usize].to_be_bytes();
                self.memory[addr] = bytes[0];
                self.memory[addr + 1] = bytes[1];
                self.memory[addr + 2] = bytes[2];
                self.memory[addr + 3] = bytes[3];
                self.pc += 1;
            }
            Instruction::Add(dst, src1, src2) => {
                let result =
                    self.registers[*src1 as usize].wrapping_add(self.registers[*src2 as usize]);
                self.registers[*dst as usize] = result;
                self.pc += 1;
            }
            Instruction::Sub(dst, src1, src2) => {
                let result =
                    self.registers[*src1 as usize].wrapping_sub(self.registers[*src2 as usize]);
                self.registers[*dst as usize] = result;
                self.pc += 1;
            }
            Instruction::Xor(dst, src1, src2) => {
                let result = self.registers[*src1 as usize] ^ self.registers[*src2 as usize];
                self.registers[*dst as usize] = result;
                self.pc += 1;
            }
            Instruction::Jmp(target) => {
                let target = *target as usize;
                if target >= instructions_len {
                    return Err(VmError::PcOutOfBounds(target));
                }
                self.pc = target;
            }
            Instruction::JmpEq(reg1, reg2, target) => {
                if self.registers[*reg1 as usize] == self.registers[*reg2 as usize] {
                    let target = *target as usize;
                    if target >= instructions_len {
                        return Err(VmError::PcOutOfBounds(target));
                    }
                    self.pc = target;
                } else {
                    self.pc += 1;
                }
            }
            Instruction::JmpNe(reg1, reg2, target) => {
                if self.registers[*reg1 as usize] != self.registers[*reg2 as usize] {
                    let target = *target as usize;
                    if target >= instructions_len {
                        return Err(VmError::PcOutOfBounds(target));
                    }
                    self.pc = target;
                } else {
                    self.pc += 1;
                }
            }
            Instruction::Halt => {
                return Ok(false);
            }
        }
        Ok(self.pc < instructions_len)
    }
}
