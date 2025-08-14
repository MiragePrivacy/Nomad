use super::*;

#[test]
fn test_basic_arithmetic() -> Result<(), VmError> {
    let mut vm = NomadVm::new(100);
    let res = vm.execute_program(program![
        Set 1, 400;
        Set 2, 200;
        Sub 0, 1, 2;
    ])?;
    assert_eq!(res[0..4], 200u32.to_be_bytes());
    Ok(())
}

#[test]
fn test_addition() -> Result<(), VmError> {
    let mut vm = NomadVm::new(100);
    let res = vm.execute_program(program![
        Set 0, 100;
        Set 1, 50;
        Add 2, 0, 1;
    ])?;
    assert_eq!(res[8..12], 150u32.to_be_bytes());
    Ok(())
}

#[test]
fn test_subtraction() -> Result<(), VmError> {
    let mut vm = NomadVm::new(100);
    let res = vm.execute_program(program![
        Set 0, 100;
        Set 1, 30;
        Sub 2, 0, 1;
    ])?;
    assert_eq!(res[8..12], 70u32.to_be_bytes());
    Ok(())
}

#[test]
fn test_xor() -> Result<(), VmError> {
    let mut vm = NomadVm::new(100);
    let res = vm.execute_program(program![
        Set 0, 0xFF00FF00;
        Set 1, 0x00FF00FF;
        Xor 2, 0, 1;
    ])?;
    assert_eq!(res[8..12], 0xFFFFFFFFu32.to_be_bytes());
    Ok(())
}

#[test]
fn test_memory_load_store() -> Result<(), VmError> {
    let mut vm = NomadVm::new(100);
    let res = vm.execute_program(program![
        Set 0, 0xDEADBEEF;
        Store 0, 1000;
        Load 1, 1000;
    ])?;
    assert_eq!(res[4..8], 0xDEADBEEFu32.to_be_bytes());
    Ok(())
}

#[test]
fn test_unconditional_jump() -> Result<(), VmError> {
    let mut vm = NomadVm::new(100);
    let res = vm.execute_program(program![
        Set 0, 1;
        Jmp 3;
        Set 0, 2;
        Set 1, 42;
    ])?;
    assert_eq!(res[0..4], 1u32.to_be_bytes());
    assert_eq!(res[4..8], 42u32.to_be_bytes());
    Ok(())
}

#[test]
fn test_conditional_jump_eq_true() -> Result<(), VmError> {
    let mut vm = NomadVm::new(100);
    let res = vm.execute_program(program![
        Set 0, 42;
        Set 1, 42;
        JmpEq 0, 1, 4;
        Set 2, 1;
        Set 2, 2;
    ])?;
    assert_eq!(res[8..12], 2u32.to_be_bytes());
    Ok(())
}

#[test]
fn test_conditional_jump_eq_false() -> Result<(), VmError> {
    let mut vm = NomadVm::new(100);
    let res = vm.execute_program(program![
        Set 0, 42;
        Set 1, 43;
        JmpEq 0, 1, 5;
        Set 2, 1;
    ])?;
    assert_eq!(res[8..12], 1u32.to_be_bytes());
    Ok(())
}

#[test]
fn test_conditional_jump_ne_true() -> Result<(), VmError> {
    let mut vm = NomadVm::new(100);
    let res = vm.execute_program(program![
        Set 0, 42;
        Set 1, 43;
        JmpNe 0, 1, 4;
        Set 2, 1;
        Set 2, 2;
    ])?;
    assert_eq!(res[8..12], 2u32.to_be_bytes());
    Ok(())
}

#[test]
fn test_conditional_jump_ne_false() -> Result<(), VmError> {
    let mut vm = NomadVm::new(100);
    let res = vm.execute_program(program![
        Set 0, 42;
        Set 1, 42;
        JmpNe 0, 1, 5;
        Set 2, 1;
    ])?;
    assert_eq!(res[8..12], 1u32.to_be_bytes());
    Ok(())
}

#[test]
fn test_halt_instruction() -> Result<(), VmError> {
    let mut vm = NomadVm::new(100);
    let res = vm.execute_program(program![
        Set 0, 123;
        Halt;
        Set 0, 456;
    ])?;
    assert_eq!(res[0..4], 123u32.to_be_bytes());
    Ok(())
}

#[test]
fn test_register_concatenation() -> Result<(), VmError> {
    let mut vm = NomadVm::new(100);
    let res = vm.execute_program(program![
        Set 0, 0x01020304;
        Set 1, 0x05060708;
        Set 2, 0x090A0B0C;
        Set 3, 0x0D0E0F10;
        Set 4, 0x11121314;
        Set 5, 0x15161718;
        Set 6, 0x191A1B1C;
        Set 7, 0x1D1E1F20;
    ])?;

    let expected = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
        0x1F, 0x20,
    ];
    assert_eq!(res, expected);
    Ok(())
}

#[test]
fn test_wrapping_arithmetic() -> Result<(), VmError> {
    let mut vm = NomadVm::new(100);
    let res = vm.execute_program(program![
        Set 0, 0xFFFFFFFF;
        Set 1, 1;
        Add 2, 0, 1;
        Set 3, 0;
        Set 4, 1;
        Sub 5, 3, 4;
    ])?;

    assert_eq!(res[8..12], 0u32.to_be_bytes());
    assert_eq!(res[20..24], 0xFFFFFFFFu32.to_be_bytes());
    Ok(())
}

#[test]
fn test_max_cycles_limit() {
    let mut vm = NomadVm::new(5);
    let res = vm.execute_program(program![
        Set 0, 1;
        Add 0, 0, 0;
        Add 0, 0, 0;
        Add 0, 0, 0;
        Add 0, 0, 0;
        Add 0, 0, 0;
        Set 1, 999;
    ]);

    assert!(res.is_ok());
    let result = res.unwrap();
    assert_eq!(result[4..8], 0u32.to_be_bytes());
}

#[test]
fn test_vm_state_reset() -> Result<(), VmError> {
    let mut vm = NomadVm::new(100);

    vm.execute_program(program![
        Set 0, 123;
        Store 0, 500;
    ])?;

    let res = vm.execute_program(program![
        Load 0, 500;
        Set 1, 456;
    ])?;
    assert_eq!(res[0..4], 0u32.to_be_bytes());
    assert_eq!(res[4..8], 456u32.to_be_bytes());

    Ok(())
}

#[test]
fn test_error_invalid_register() {
    let bytecode = vec![0x00, 8, 0, 0, 0, 42];
    let mut vm = NomadVm::new(100);
    let result = vm.execute(bytecode);
    assert!(matches!(result, Err(VmError::InvalidRegister(8))));
}

#[test]
fn test_error_memory_out_of_bounds_load() {
    let mut vm = NomadVm::new(100);
    let result = vm.execute_program(program![
        Load 0, 0xFFFFFFFF;
    ]);
    assert!(matches!(result, Err(VmError::MemoryOutOfBounds(_))));
}

#[test]
fn test_error_memory_out_of_bounds_store() {
    let mut vm = NomadVm::new(100);
    let result = vm.execute_program(program![
        Store 0, 0xFFFFFFFF;
    ]);
    assert!(matches!(result, Err(VmError::MemoryOutOfBounds(_))));
}

#[test]
fn test_error_pc_out_of_bounds_jmp() {
    let mut vm = NomadVm::new(100);
    let result = vm.execute_program(program![
        Jmp 999;
    ]);
    assert!(matches!(result, Err(VmError::PcOutOfBounds(999))));
}

#[test]
fn test_error_pc_out_of_bounds_jmp_eq() {
    let mut vm = NomadVm::new(100);
    let result = vm.execute_program(program![
        Set 0, 1;
        Set 1, 1;
        JmpEq 0, 1, 999;
    ]);
    assert!(matches!(result, Err(VmError::PcOutOfBounds(999))));
}

#[test]
fn test_error_pc_out_of_bounds_jmp_ne() {
    let mut vm = NomadVm::new(100);
    let result = vm.execute_program(program![
        Set 0, 1;
        Set 1, 2;
        JmpNe 0, 1, 999;
    ]);
    assert!(matches!(result, Err(VmError::PcOutOfBounds(999))));
}

#[test]
fn test_error_invalid_instruction() {
    let bytecode = vec![0x99];
    let mut vm = NomadVm::new(100);
    let result = vm.execute(bytecode);
    assert!(matches!(result, Err(VmError::InvalidInstruction(0x99))));
}

#[test]
fn test_empty_program() {
    let mut vm = NomadVm::new(100);
    let result = vm.execute_program(program![]);
    assert!(result.is_ok());
    let res = result.unwrap();
    assert_eq!(res, [0u8; 32]);
}

#[test]
fn test_error_invalid_program_truncated() {
    let bytecode = vec![0x00, 0];
    let mut vm = NomadVm::new(100);
    let result = vm.execute(bytecode);
    assert!(matches!(result, Err(VmError::InvalidProgram)));
}

#[test]
fn test_program_encode_decode_roundtrip() -> Result<(), VmError> {
    let original_program = program![
        Set 0, 0x12345678;
        Set 1, 0x87654321;
        Add 2, 0, 1;
        Store 2, 1000;
        Load 3, 1000;
        JmpEq 2, 3, 7;
        Set 4, 0xFFFFFFFF;
        Halt;
    ];

    let mut bytecode = Vec::new();
    original_program.encode(&mut bytecode).unwrap();

    let decoded_program = Program::from_bytes(&bytecode)?;

    let mut vm1 = NomadVm::new(100);
    let mut vm2 = NomadVm::new(100);

    let result1 = vm1.execute_program(original_program)?;
    let result2 = vm2.execute_program(decoded_program)?;

    assert_eq!(result1, result2);
    Ok(())
}

#[test]
fn test_complex_program() -> Result<(), VmError> {
    let mut vm = NomadVm::new(1000);
    let res = vm.execute_program(program![
        Set 0, 10;
        Set 1, 20;
        Add 2, 0, 1;
        Sub 3, 2, 0;
        Xor 4, 1, 3;
    ])?;

    assert_eq!(res[8..12], 30u32.to_be_bytes());
    assert_eq!(res[12..16], 20u32.to_be_bytes());
    assert_eq!(res[16..20], 0u32.to_be_bytes());
    Ok(())
}
