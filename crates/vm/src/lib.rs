use affair::{DedicatedThread, Executor, Socket, Worker};
use thiserror::Error;
use tracing::{instrument, trace, Span};

pub use crate::ops::*;
pub use crate::program::*;

mod ops;
mod program;
#[cfg(test)]
mod tests;

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
    #[error("Memory address out of bounds: {0:08X}")]
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
/// - Configurable max cycle count
/// - 256-bit program output concatinated from registers
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
        let program = Program::from_bytes(&bytecode)?;
        self.execute_program(program)
    }

    /// Executes a program, resets, and returns the result from the concatinated registers.
    pub fn execute_program(&mut self, program: Program) -> Result<[u8; 32], VmError> {
        // Execute instructions
        let mut cycles = 0;
        while let Some(instruction) = program.get(self.pc) {
            if let Err(e) = self.execute_instruction(instruction, program.len()) {
                println!("{e} - {}", self.pc);
                return Err(e);
            }
            cycles += 1;
            if cycles > self.max_cycles || instruction == &Instruction::Halt() {
                break;
            }
        }

        // Compute result from register values
        let mut result = [0u8; 32];
        for (i, val) in self.registers.iter().take(8).enumerate() {
            let offset = i * 4;
            result[offset..offset + 4].copy_from_slice(&val.to_be_bytes());
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
    ) -> Result<(), VmError> {
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
            Instruction::Print(bitmap) => {
                #[cfg(debug_assertions)]
                {
                    print!("DEBUG: ");
                    let mut first = true;
                    for reg_idx in 0..8u8 {
                        if (bitmap & (1 << reg_idx)) != 0 {
                            if !first {
                                print!(", ");
                            }
                            print!("R{}: 0x{:08X}", reg_idx, self.registers[reg_idx as usize]);
                            first = false;
                        }
                    }
                    println!();
                }
                #[cfg(not(debug_assertions))]
                let _ = bitmap;
                self.pc += 1;
            }
            Instruction::Halt() => {}
        }
        Ok(())
    }
}
