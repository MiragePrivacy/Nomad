use std::{
    io::{Result as IoResult, Write},
    ops::Deref,
};

use crate::{Instruction, VmError};

/// Construct an unvalidated program from raw mnemonics
///
/// # Safety
///
/// Executing a manually assembled program may result in undefined behavior
///
/// # Example
///
/// ```ignore
/// nomad_vm::program![
///     // setup two registers
///     Set 1, 400;
///     Set 2, 200;
///     // reg0 = reg1 - reg2
///     Sub 0, 1, 2;
///     // store 32-bit result in memory
///     Store 0, 500
/// ]
/// ```
#[macro_export]
macro_rules! program {
    [$( $op:ident $($arg:expr),* ; )*] => {
        $crate::Program::from_raw(vec![
            $( $crate::Instruction::$op( $($arg),* ) ),*
        ])
    };
}

pub struct Program(Vec<Instruction>);

impl Deref for Program {
    type Target = [Instruction];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Program {
    /// Construct an unvalidated program from instructions.
    ///
    /// # Safety
    ///
    /// Executing a manually assembled program may result in undefined behavior
    pub fn from_raw(ops: Vec<Instruction>) -> Self {
        Self(ops)
    }

    /// Parse and validate program bytecode into a list of instructions.
    pub fn from_bytes(bytes: &[u8]) -> Result<Program, VmError> {
        let mut instructions = Vec::new();
        let mut offset = 0;
        while offset < bytes.len() {
            let (instruction, size) = Instruction::from_bytes(&bytes[offset..])?;
            instructions.push(instruction);
            offset += size;
        }
        Ok(Program(instructions))
    }

    /// Write the program bytecode into a given buffer.
    pub fn encode<W: Write>(&self, writer: &mut W) -> IoResult<()> {
        for instruction in &self.0 {
            instruction.encode(writer)?;
        }
        Ok(())
    }
}
