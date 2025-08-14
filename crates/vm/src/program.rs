use crate::{Instruction, VmError};
use std::io::{Result as IoResult, Write};

pub struct Program(pub(crate) Vec<Instruction>);

impl Program {
    /// Construct and unvalidated program from instructions
    pub fn from_raw(instructions: Vec<Instruction>) -> Self {
        Self(instructions)
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

#[macro_export]
macro_rules! program {
    [$( $op:tt $($arg:expr),* ;)*] => {
        $crate::Program::from_raw(vec![
            $( $crate::Instruction::$op( $($arg),* ) ),*
        ])
    };
}

#[test]
fn test_program() {
    let program = program![
        Set 0, 400;
        Set 1, 200;
        Sub 2, 0, 1;
    ];
}
