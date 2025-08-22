# Puzzle Generator

A sophisticated puzzle generator that creates complex VM programs designed to prevent static analysis while producing deterministic 256-bit outputs.

## Overview

The puzzle generator creates programs for the Nomad VM that:

- **Prevent Static Analysis**: Uses variable jumps, memory scrambling, and opaque code paths
- **Enable Dynamic Execution**: Programs must be executed to determine their output
- **Produce Deterministic Results**: Same puzzle always produces same 256-bit output
- **Use Complex Transformations**: Multiple layers of arithmetic, memory, and control flow operations

### Transformation Types

#### ArithmeticChain
Complex sequences of ADD, SUB, and XOR operations across multiple registers.
- Creates arithmetic dependencies between registers
- Uses varying operation sequences to obscure computation
- Intermixes operations across different register sets

#### MemoryScramble
Random memory read/write patterns across the 1GB memory space.
- Stores and loads data at unpredictable addresses
- Uses bit patterns to determine read vs write operations
- XOR operations between memory data to scramble values

#### ConditionalJump
Variable conditional jumps creating complex control flow.
- Uses JMPEQ and JMPNE instructions with register comparisons
- Multiple possible jump targets create branching paths
- Modifies condition registers to create complex execution flows

#### EncryptionRound
XOR-based encryption using keys stored in memory.
- Loads encryption keys from randomized memory locations
- Applies multiple encryption rounds to all registers
- Rotates keys between rounds for additional complexity

#### RegisterShuffle
Unpredictable register value movement patterns.
- Maps source registers to different destination registers
- Uses memory as temporary storage to avoid conflicts
- Creates data dependencies across all 8 VM registers

## Generated Program Features

### Security Properties
- **No Static Solution**: Programs must be executed to determine output
- **Variable Control Flow**: Conditional jumps prevent linear analysis
- **Memory Obfuscation**: Data scattered across 1GB address space
- **Register Dependencies**: All 8 registers interdependent
- **Dead Code Injection**: Unreachable instructions confuse disassemblers

### VM Compatibility
- Uses only standard Nomad VM instruction set
- Respects 1GB memory limit and register constraints
- Terminates with HALT instruction
- Produces valid 256-bit output in registers R0-R7

## Implementation Details

### Recursive Transformation Building
The generator works backwards from the target output:

1. **Target State**: Convert 256-bit output to register values
2. **Transform Selection**: Choose random transformation types
3. **Input Generation**: Create random input states for each transformation
4. **Recursive Application**: Apply transformations to input states
5. **Depth Limiting**: Stop recursion at maximum depth

### Instruction Compilation
The compiler converts transformations to VM instructions:

1. **Memory Initialization**: Set up encryption keys and random data
2. **Transformation Compilation**: Convert each transformation to instruction sequence
3. **Obfuscation Injection**: Add dead code and noise operations between transformations
4. **Register Finalization**: Ensure final state produces target output
5. **Program Termination**: Add HALT instruction

### Instruction Count Management
The system tracks instruction generation to stay within limits:
- Each transformation estimates its instruction count
- Compiler aborts if maximum would be exceeded
- Obfuscation is scaled based on remaining instruction budget

## Future Enhancements

- **Inverse Computation**: Proper backwards computation from target state
- **Optimization Passes**: Reduce instruction count while maintaining complexity
- **Additional Transformations**: More obfuscation techniques
- **Parallel Generation**: Multi-threaded puzzle creation
- **Verification**: Execute generated puzzles to verify correctness
