use crate::{VmError, REGISTERS};
use std::io::{Result as IoResult, Write};

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum Opcode {
    Set = 0x00,
    Load = 0x01,
    Store = 0x02,
    Add = 0x03,
    Sub = 0x04,
    Xor = 0x05,
    Jmp = 0x06,
    JmpEq = 0x07,
    JmpNe = 0x08,
    Halt = 0xFF,
}

impl Opcode {
    pub fn size(&self) -> usize {
        match self {
            Opcode::Set => 1 + 1 + 4,       // opcode + reg + value
            Opcode::Load => 1 + 1 + 4,      // opcode + reg + addr
            Opcode::Store => 1 + 1 + 4,     // opcode + reg + addr
            Opcode::Add => 1 + 1 + 1 + 1,   // opcode + dst_reg + src1_reg + src2_reg
            Opcode::Sub => 1 + 1 + 1 + 1,   // opcode + dst_reg + src1_reg + src2_reg
            Opcode::Xor => 1 + 1 + 1 + 1,   // opcode + dst_reg + src1_reg + src2_reg
            Opcode::Jmp => 1 + 4,           // opcode + target
            Opcode::JmpEq => 1 + 1 + 1 + 4, // opcode + reg1 + reg2 + target
            Opcode::JmpNe => 1 + 1 + 1 + 4, // opcode + reg1 + reg2 + target
            Opcode::Halt => 1,              // opcode only
        }
    }
}

impl TryFrom<u8> for Opcode {
    type Error = VmError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(Opcode::Set),
            0x01 => Ok(Opcode::Load),
            0x02 => Ok(Opcode::Store),
            0x03 => Ok(Opcode::Add),
            0x04 => Ok(Opcode::Sub),
            0x05 => Ok(Opcode::Xor),
            0x06 => Ok(Opcode::Jmp),
            0x07 => Ok(Opcode::JmpEq),
            0x08 => Ok(Opcode::JmpNe),
            0xFF => Ok(Opcode::Halt),
            _ => Err(VmError::InvalidInstruction(value)),
        }
    }
}

/// Virtual machine instruction set.
///
/// Each instruction operates on 8 registers (0-7) and 1GiB of memory space.
/// Instructions use big-endian encoding for multi-byte values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    /// Assign a constant value to a register.
    ///
    /// # Arguments
    /// * `u8` - Target register (0-7)
    /// * `u32` - Constant value to assign
    ///
    /// # Example
    /// ```ignore
    /// Assign(0, 42) // Sets register 0 to 42
    /// ```
    Set(u8, u32),

    /// Load a 32-bit value from memory into a register.
    ///
    /// # Arguments
    /// * `u8` - Target register (0-7)
    /// * `u32` - Memory address to load from
    ///
    /// # Example
    /// ```ignore
    /// Load(1, 0x1000) // Loads 4 bytes from address 0x1000 into register 1
    /// ```
    Load(u8, u32),

    /// Store a register's value to memory.
    ///
    /// # Arguments
    /// * `u8` - Source register (0-7)
    /// * `u32` - Memory address to store to
    ///
    /// # Example
    /// ```ignore
    /// Store(2, 0x2000) // Stores register 2's value to address 0x2000
    /// ```
    Store(u8, u32),

    /// Add two registers and store the result.
    ///
    /// # Arguments
    /// * `u8` - Destination register (0-7)
    /// * `u8` - First source register (0-7)
    /// * `u8` - Second source register (0-7)
    ///
    /// # Example
    /// ```ignore
    /// Add(0, 1, 2) // reg[0] = reg[1] + reg[2]
    /// ```
    Add(u8, u8, u8),

    /// Subtract two registers and store the result.
    ///
    /// # Arguments
    /// * `u8` - Destination register (0-7)
    /// * `u8` - First source register (0-7)
    /// * `u8` - Second source register (0-7)
    ///
    /// # Example
    /// ```ignore
    /// Sub(0, 1, 2) // reg[0] = reg[1] - reg[2]
    /// ```
    Sub(u8, u8, u8),

    /// XOR two registers and store the result.
    ///
    /// # Arguments
    /// * `u8` - Destination register (0-7)
    /// * `u8` - First source register (0-7)
    /// * `u8` - Second source register (0-7)
    ///
    /// # Example
    /// ```ignore
    /// Xor(0, 1, 2) // reg[0] = reg[1] ^ reg[2]
    /// ```
    Xor(u8, u8, u8),

    /// Unconditional jump to a target instruction.
    ///
    /// # Arguments
    /// * `u32` - Target instruction index (not byte offset)
    ///
    /// # Example
    /// ```ignore
    /// Jmp(10) // Jump to instruction 10
    /// ```
    Jmp(u32),

    /// Conditional jump if two registers are equal.
    ///
    /// # Arguments
    /// * `u8` - First register to compare (0-7)
    /// * `u8` - Second register to compare (0-7)
    /// * `u32` - Target instruction index if equal
    ///
    /// # Example
    /// ```ignore
    /// JmpEq(0, 1, 5) // Jump to instruction 5 if reg[0] == reg[1]
    /// ```
    JmpEq(u8, u8, u32),

    /// Conditional jump if two registers are not equal.
    ///
    /// # Arguments
    /// * `u8` - First register to compare (0-7)
    /// * `u8` - Second register to compare (0-7)
    /// * `u32` - Target instruction index if not equal
    ///
    /// # Example
    /// ```ignore
    /// JmpNe(0, 1, 5) // Jump to instruction 5 if reg[0] != reg[1]
    /// ```
    JmpNe(u8, u8, u32),

    /// Halt program execution.
    ///
    /// Stops the VM and returns the current register state.
    /// No arguments required.
    ///
    /// # Example
    /// ```ignore
    /// Halt // Stop execution
    /// ```
    Halt(),
}

fn validate_reg(reg: u8) -> Result<u8, VmError> {
    if reg >= REGISTERS as u8 {
        Err(VmError::InvalidRegister(reg))
    } else {
        Ok(reg)
    }
}

impl Instruction {
    pub fn size(&self) -> usize {
        match self {
            Instruction::Set { .. } => 6,
            Instruction::Load { .. } => 6,
            Instruction::Store { .. } => 6,
            Instruction::Add { .. } => 4,
            Instruction::Sub { .. } => 4,
            Instruction::Xor { .. } => 4,
            Instruction::Jmp { .. } => 5,
            Instruction::JmpEq { .. } => 7,
            Instruction::JmpNe { .. } => 7,
            Instruction::Halt() => 1,
        }
    }

    /// Parse an instruction from a byte slice, returning the instruction
    /// and number of bytes consumed if valid.
    pub fn from_bytes(bytes: &[u8]) -> Result<(Self, usize), VmError> {
        if bytes.is_empty() {
            return Err(VmError::InvalidProgram);
        }
        let opcode = Opcode::try_from(bytes[0])?;

        let required_size = opcode.size();
        if bytes.len() < required_size {
            return Err(VmError::InvalidProgram);
        }

        let instruction = match opcode {
            Opcode::Set => Instruction::Set(
                validate_reg(bytes[1])?,
                u32::from_be_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
            ),
            Opcode::Load => Instruction::Load(
                validate_reg(bytes[1])?,
                u32::from_be_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
            ),
            Opcode::Store => Instruction::Store(
                validate_reg(bytes[1])?,
                u32::from_be_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
            ),
            Opcode::Add => Instruction::Add(
                validate_reg(bytes[1])?,
                validate_reg(bytes[2])?,
                validate_reg(bytes[3])?,
            ),
            Opcode::Sub => Instruction::Sub(
                validate_reg(bytes[1])?,
                validate_reg(bytes[2])?,
                validate_reg(bytes[3])?,
            ),
            Opcode::Xor => Instruction::Xor(
                validate_reg(bytes[1])?,
                validate_reg(bytes[2])?,
                validate_reg(bytes[3])?,
            ),
            Opcode::Jmp => {
                Instruction::Jmp(u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]))
            }
            Opcode::JmpEq => Instruction::JmpEq(
                validate_reg(bytes[1])?,
                validate_reg(bytes[2])?,
                u32::from_be_bytes([bytes[3], bytes[4], bytes[5], bytes[6]]),
            ),
            Opcode::JmpNe => Instruction::JmpNe(
                validate_reg(bytes[1])?,
                validate_reg(bytes[2])?,
                u32::from_be_bytes([bytes[3], bytes[4], bytes[5], bytes[6]]),
            ),
            Opcode::Halt => Instruction::Halt(),
        };

        Ok((instruction, required_size))
    }

    /// Encode the instruction and write it to a given buffer
    pub fn encode<W: Write>(&self, writer: &mut W) -> IoResult<()> {
        match self {
            Instruction::Set(reg, value) => {
                let mut buf = [0u8; 6];
                buf[0] = Opcode::Set as u8;
                buf[1] = *reg;
                buf[2..6].copy_from_slice(&value.to_be_bytes());
                writer.write_all(&buf)?;
            }
            Instruction::Load(reg, addr) => {
                let mut buf = [0u8; 6];
                buf[0] = Opcode::Load as u8;
                buf[1] = *reg;
                buf[2..6].copy_from_slice(&addr.to_be_bytes());
                writer.write_all(&buf)?;
            }
            Instruction::Store(reg, addr) => {
                let mut buf = [0u8; 6];
                buf[0] = Opcode::Store as u8;
                buf[1] = *reg;
                buf[2..6].copy_from_slice(&addr.to_be_bytes());
                writer.write_all(&buf)?;
            }
            Instruction::Add(dst, src1, src2) => {
                let buf = [Opcode::Add as u8, *dst, *src1, *src2];
                writer.write_all(&buf)?;
            }
            Instruction::Sub(dst, src1, src2) => {
                let buf = [Opcode::Sub as u8, *dst, *src1, *src2];
                writer.write_all(&buf)?;
            }
            Instruction::Xor(dst, src1, src2) => {
                let buf = [Opcode::Xor as u8, *dst, *src1, *src2];
                writer.write_all(&buf)?;
            }
            Instruction::Jmp(target) => {
                let mut buf = [0u8; 5];
                buf[0] = Opcode::Jmp as u8;
                buf[1..5].copy_from_slice(&target.to_be_bytes());
                writer.write_all(&buf)?;
            }
            Instruction::JmpEq(reg1, reg2, target) => {
                let mut buf = [0u8; 7];
                buf[0] = Opcode::JmpEq as u8;
                buf[1] = *reg1;
                buf[2] = *reg2;
                buf[3..7].copy_from_slice(&target.to_be_bytes());
                writer.write_all(&buf)?;
            }
            Instruction::JmpNe(reg1, reg2, target) => {
                let mut buf = [0u8; 7];
                buf[0] = Opcode::JmpNe as u8;
                buf[1] = *reg1;
                buf[2] = *reg2;
                buf[3..7].copy_from_slice(&target.to_be_bytes());
                writer.write_all(&buf)?;
            }
            Instruction::Halt() => {
                writer.write_all(&[Opcode::Halt as u8])?;
            }
        }
        Ok(())
    }
}
