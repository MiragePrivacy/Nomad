use crate::{ArithmeticOp, Transformation};
use rand::Rng;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct NodeId(usize);

impl NodeId {
    pub fn new(id: usize) -> Self {
        Self(id)
    }

    pub fn inner(&self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone)]
pub struct TransformationNode {
    pub id: NodeId,
    pub input_count: usize,
    pub output_count: usize,
    pub operation: Transformation,
    /// Set of VM registers this node is allowed to use (0-7)
    pub assigned_registers: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Connection {
    pub from_node: NodeId,
    pub from_output: usize,
    pub to_node: NodeId,
    pub to_input: usize,
}

#[derive(Debug)]
pub struct TransformationChain {
    pub nodes: HashMap<NodeId, TransformationNode>,
    pub connections: Vec<Connection>,
    pub entry_nodes: Vec<NodeId>,
    pub exit_nodes: Vec<NodeId>,
    next_id: usize,
}

impl TransformationChain {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            connections: Vec::new(),
            entry_nodes: Vec::new(),
            exit_nodes: Vec::new(),
            next_id: 0,
        }
    }

    pub fn add_node(&mut self, mut node: TransformationNode) -> NodeId {
        let id = NodeId::new(self.next_id);
        self.next_id += 1;
        node.id = id;
        self.nodes.insert(id, node);
        id
    }

    pub fn connect(
        &mut self,
        from_node: NodeId,
        from_output: usize,
        to_node: NodeId,
        to_input: usize,
    ) {
        self.connections.push(Connection {
            from_node,
            from_output,
            to_node,
            to_input,
        });
    }

    pub fn next_node_id(&self) -> NodeId {
        NodeId::new(self.next_id)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Generate Mermaid diagram for the transformation chain
    pub fn to_mermaid_string(&self) -> String {
        let mut diagram = String::new();

        // Header
        diagram.push_str("graph TD\n");

        // Generate nodes with register information in sorted order
        let mut sorted_nodes: Vec<_> = self.nodes.iter().collect();
        sorted_nodes.sort_by_key(|(node_id, _)| *node_id);

        for (node_id, node) in sorted_nodes {
            let node_name = format!("N{}", node_id.inner());
            let registers_info = format_register_list(&node.assigned_registers);

            let label = match &node.operation {
                Transformation::Split {} => {
                    format!(
                        "Split<br/>{}→{} branches<br/>{}",
                        node.input_count, node.output_count, registers_info
                    )
                }

                Transformation::Rejoin {} => {
                    format!(
                        "Rejoin<br/>{}→{}<br/>{}", 
                        node.input_count, node.output_count, registers_info
                    )
                }

                Transformation::ArithmeticChain {
                    operations,
                    registers,
                } => {
                    let ops = operations
                        .iter()
                        .map(|op| match op {
                            ArithmeticOp::Add => "+",
                            ArithmeticOp::Sub => "-",
                            ArithmeticOp::Xor => "⊕",
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    let reg_list = registers
                        .iter()
                        .map(|r| format!("R{r}"))
                        .collect::<Vec<_>>()
                        .join(",");
                    format!("Arithmetic<br/>{ops}<br/>on [{reg_list}]<br/>{registers_info}")
                }

                Transformation::MemoryScramble { addresses, pattern } => {
                    format!(
                        "Memory<br/>{} addrs<br/>pattern: 0x{:04X}<br/>{}",
                        addresses.len(),
                        pattern & 0xFFFF,
                        registers_info
                    )
                }

                Transformation::ConditionalJump {
                    condition_regs,
                    jump_targets,
                } => {
                    format!(
                        "Jump<br/>R{}≠R{}?<br/>{} targets<br/>{}", 
                        condition_regs.0,
                        condition_regs.1,
                        jump_targets.len(),
                        registers_info
                    )
                }

                Transformation::EncryptionRound { rounds, key_addr } => {
                    format!(
                        "Encrypt<br/>{rounds} rounds<br/>key@0x{key_addr:X}<br/>{registers_info}"
                    )
                }

                Transformation::RegisterShuffle { mapping } => {
                    let shuffles = mapping
                        .iter()
                        .take(3)
                        .map(|(src, dst)| format!("R{src}→R{dst}"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let more = if mapping.len() > 3 { "..." } else { "" };
                    format!("Shuffle<br/>{shuffles}{more}<br/>{registers_info}")
                }
            };

            diagram.push_str(&format!("  {node_name}[\"{label}\"]\n"));
        }

        diagram.push('\n');

        // Generate connections - only unique node pairs
        let mut unique_connections = std::collections::BTreeSet::new();
        for connection in &self.connections {
            unique_connections.insert((connection.from_node, connection.to_node));
        }

        for (from_node, to_node) in unique_connections {
            diagram.push_str(&format!(
                "  N{} --> N{}\n",
                from_node.inner(),
                to_node.inner()
            ));
        }

        diagram
    }
}

/// Format register list for display in mermaid nodes
fn format_register_list(registers: &[u8]) -> String {
    if registers.is_empty() {
        return "No regs".to_string();
    }

    if registers.len() <= 4 {
        format!(
            "Regs: [{}]",
            registers
                .iter()
                .map(|r| format!("R{r}"))
                .collect::<Vec<_>>()
                .join(",")
        )
    } else {
        format!(
            "Regs: [R{},R{}...R{}] ({})",
            registers[0],
            registers[1],
            registers[registers.len() - 1],
            registers.len()
        )
    }
}

impl Default for TransformationChain {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ChainBuilder<R: Rng> {
    max_depth: usize,
    max_splits: usize,
    split_probability: f32,
    rejoin_probability: f32,
    rng: R,
}

impl<R: Rng> ChainBuilder<R> {
    pub fn new(max_depth: usize, rng: R) -> Self {
        Self {
            max_depth,
            max_splits: 4,
            split_probability: 0.5,
            rejoin_probability: 0.2,
            rng,
        }
    }

    /// Allocate disjoint register sets for split branches
    fn allocate_branch_registers(
        &mut self,
        available_registers: &[u8],
        branch_count: usize,
    ) -> Vec<Vec<u8>> {
        if available_registers.is_empty() || branch_count == 0 {
            return vec![Vec::new(); branch_count];
        }

        let registers_per_branch = available_registers.len() / branch_count;
        let remainder = available_registers.len() % branch_count;

        let mut allocated = Vec::new();
        let mut reg_index = 0;

        for i in 0..branch_count {
            let mut branch_size = registers_per_branch;
            if i < remainder {
                branch_size += 1; // Distribute remainder registers to first branches
            }

            let mut branch_regs = Vec::new();
            for _ in 0..branch_size {
                if reg_index < available_registers.len() {
                    branch_regs.push(available_registers[reg_index]);
                    reg_index += 1;
                }
            }

            // Ensure each branch has at least one register if possible
            if branch_regs.is_empty() && !available_registers.is_empty() {
                branch_regs.push(available_registers[i % available_registers.len()]);
            }

            allocated.push(branch_regs);
        }

        allocated
    }

    /// Merge register sets from multiple branches back together
    fn merge_branch_registers(&self, branch_registers: &[Vec<u8>]) -> Vec<u8> {
        let mut merged = HashSet::new();
        for branch_regs in branch_registers {
            merged.extend(branch_regs.iter().copied());
        }
        let mut result: Vec<u8> = merged.into_iter().collect();
        result.sort();
        result
    }

    pub fn with_split_probability(mut self, probability: f32) -> Self {
        self.split_probability = probability.clamp(0.0, 1.0);
        self
    }

    pub fn with_rejoin_probability(mut self, probability: f32) -> Self {
        self.rejoin_probability = probability.clamp(0.0, 1.0);
        self
    }

    pub fn with_max_splits(mut self, max_splits: usize) -> Self {
        self.max_splits = max_splits.max(2);
        self
    }

    pub fn build_chain(&mut self, input_count: usize, output_count: usize) -> TransformationChain {
        let mut chain = TransformationChain::new();

        // Create entry point with all 8 VM registers available
        let all_registers: Vec<u8> = (0..8).collect();
        let entry_node = TransformationNode {
            id: NodeId::new(0), // Will be updated by add_node
            input_count,
            output_count: input_count,
            operation: Transformation::ArithmeticChain {
                operations: vec![ArithmeticOp::Add],
                registers: vec![0],
            },
            assigned_registers: all_registers.clone(),
        };
        let entry_id = chain.add_node(entry_node);
        chain.entry_nodes.push(entry_id);

        // Track current layer with (node_id, output_count, assigned_registers)
        let mut current_layer = vec![(entry_id, input_count, all_registers)];

        // Build layers with splits and transforms
        for depth in 0..self.max_depth {
            current_layer = self.build_layer(&mut chain, current_layer, depth);
        }

        // Create final convergence to desired output count
        let exit_id = self.create_convergence_node(&mut chain, current_layer, output_count);
        chain.exit_nodes.push(exit_id);

        chain
    }

    fn build_layer(
        &mut self,
        chain: &mut TransformationChain,
        input_layer: Vec<(NodeId, usize, Vec<u8>)>,
        depth: usize,
    ) -> Vec<(NodeId, usize, Vec<u8>)> {
        let mut output_layer = Vec::new();

        for (node_id, node_outputs, assigned_registers) in input_layer {
            if self.should_split(depth) && node_outputs > 1 {
                // Create split node with register isolation
                let split_nodes =
                    self.create_split_chain(chain, node_id, node_outputs, assigned_registers);
                output_layer.extend(split_nodes);
            } else {
                // Create simple transformation
                let transform_node =
                    self.create_transform_node(chain, node_id, node_outputs, assigned_registers);
                output_layer.push(transform_node);
            }
        }

        // Potentially rejoin some parallel paths
        if output_layer.len() > 1 && self.should_rejoin(depth) {
            output_layer = self.create_rejoin_opportunities(chain, output_layer);
        }

        output_layer
    }

    fn create_split_chain(
        &mut self,
        chain: &mut TransformationChain,
        source_node: NodeId,
        input_count: usize,
        available_registers: Vec<u8>,
    ) -> Vec<(NodeId, usize, Vec<u8>)> {
        let split_count = self
            .rng
            .random_range(2..=self.max_splits.min(input_count * 2));

        // Allocate disjoint register sets for each branch
        let branch_registers = self.allocate_branch_registers(&available_registers, split_count);

        let split_node = TransformationNode {
            id: NodeId::new(0), // Will be updated by add_node
            input_count,
            output_count: split_count,
            operation: Transformation::Split {},
            assigned_registers: available_registers, // Split node uses all available registers
        };
        let split_id = chain.add_node(split_node);

        // Connect source to split
        chain.connect(source_node, 0, split_id, 0);

        // Create transformation nodes for each split output with isolated registers
        let mut result = Vec::new();
        for i in 0..split_count {
            let transform_outputs = self.rng.random_range(1..=input_count);
            let branch_regs = branch_registers[i].clone();

            let transform_node = TransformationNode {
                id: NodeId::new(0), // Will be updated by add_node
                input_count: 1,     // Each split output
                output_count: transform_outputs,
                operation: self.generate_random_transform_with_registers(
                    1,
                    transform_outputs,
                    &branch_regs,
                ),
                assigned_registers: branch_regs.clone(),
            };
            let transform_id = chain.add_node(transform_node);

            chain.connect(split_id, i, transform_id, 0);
            result.push((transform_id, transform_outputs, branch_regs));
        }

        result
    }

    fn create_transform_node(
        &mut self,
        chain: &mut TransformationChain,
        source_node: NodeId,
        input_count: usize,
        assigned_registers: Vec<u8>,
    ) -> (NodeId, usize, Vec<u8>) {
        let output_count = self.rng.random_range(1..=input_count.max(1) * 2);
        let transform_node = TransformationNode {
            id: NodeId::new(0), // Will be updated by add_node
            input_count,
            output_count,
            operation: self.generate_random_transform_with_registers(
                input_count,
                output_count,
                &assigned_registers,
            ),
            assigned_registers: assigned_registers.clone(),
        };
        let transform_id = chain.add_node(transform_node);

        chain.connect(source_node, 0, transform_id, 0);
        (transform_id, output_count, assigned_registers)
    }

    fn create_rejoin_opportunities(
        &mut self,
        chain: &mut TransformationChain,
        mut parallel_nodes: Vec<(NodeId, usize, Vec<u8>)>,
    ) -> Vec<(NodeId, usize, Vec<u8>)> {
        if parallel_nodes.len() < 2 {
            return parallel_nodes;
        }

        let rejoin_count = self.rng.random_range(1..=parallel_nodes.len() / 2);
        let mut result = Vec::new();

        for _ in 0..rejoin_count {
            if parallel_nodes.len() < 2 {
                break;
            }

            // Select random nodes to rejoin
            let rejoin_size = self.rng.random_range(2..=parallel_nodes.len().min(4));
            let mut inputs_to_rejoin = Vec::new();

            for _ in 0..rejoin_size {
                if !parallel_nodes.is_empty() {
                    let idx = self.rng.random_range(0..parallel_nodes.len());
                    inputs_to_rejoin.push(parallel_nodes.remove(idx));
                }
            }

            if inputs_to_rejoin.len() >= 2 {
                let rejoin_node = self.create_rejoin_node(chain, inputs_to_rejoin);
                result.push(rejoin_node);
            }
        }

        // Add remaining non-rejoined nodes
        result.extend(parallel_nodes);
        result
    }

    fn create_rejoin_node(
        &mut self,
        chain: &mut TransformationChain,
        inputs: Vec<(NodeId, usize, Vec<u8>)>,
    ) -> (NodeId, usize, Vec<u8>) {
        let total_inputs: usize = inputs.iter().map(|(_, count, _)| count).sum();
        let output_count = self.rng.random_range(1..=total_inputs);

        // Merge all register sets from the branches being rejoined
        let branch_registers: Vec<Vec<u8>> =
            inputs.iter().map(|(_, _, regs)| regs.clone()).collect();
        let merged_registers = self.merge_branch_registers(&branch_registers);

        let rejoin_node = TransformationNode {
            id: NodeId::new(0), // Will be updated by add_node
            input_count: total_inputs,
            output_count,
            operation: Transformation::Rejoin {},
            assigned_registers: merged_registers.clone(),
        };
        let rejoin_id = chain.add_node(rejoin_node);

        // Connect all inputs to rejoin node
        let mut input_offset = 0;
        for (source_node, source_outputs, _) in inputs {
            for output_idx in 0..source_outputs {
                chain.connect(source_node, output_idx, rejoin_id, input_offset);
                input_offset += 1;
            }
        }

        (rejoin_id, output_count, merged_registers)
    }

    fn create_convergence_node(
        &mut self,
        chain: &mut TransformationChain,
        inputs: Vec<(NodeId, usize, Vec<u8>)>,
        target_output_count: usize,
    ) -> NodeId {
        if inputs.len() == 1 && inputs[0].1 == target_output_count {
            // Already the right size, just return the existing node
            return inputs[0].0;
        }

        let total_inputs: usize = inputs.iter().map(|(_, count, _)| count).sum();

        // Merge all register sets for final convergence
        let branch_registers: Vec<Vec<u8>> =
            inputs.iter().map(|(_, _, regs)| regs.clone()).collect();
        let merged_registers = self.merge_branch_registers(&branch_registers);

        let convergence_node = TransformationNode {
            id: NodeId::new(0), // Will be updated by add_node
            input_count: total_inputs,
            output_count: target_output_count,
            operation: Transformation::Rejoin {},
            assigned_registers: merged_registers,
        };
        let convergence_id = chain.add_node(convergence_node);

        // Connect all inputs to convergence node
        let mut input_offset = 0;
        for (source_node, source_outputs, _) in inputs {
            for output_idx in 0..source_outputs {
                chain.connect(source_node, output_idx, convergence_id, input_offset);
                input_offset += 1;
            }
        }

        convergence_id
    }

    fn generate_random_transform_with_registers(
        &mut self,
        input_count: usize,
        _output_count: usize,
        assigned_registers: &[u8],
    ) -> Transformation {
        // Ensure we use only assigned registers for transformations that affect registers
        match self.rng.random_range(0..5) {
            0 => Transformation::ArithmeticChain {
                operations: self.generate_arithmetic_ops(),
                registers: self.generate_constrained_register_set(assigned_registers, input_count),
            },
            1 => Transformation::MemoryScramble {
                addresses: self.generate_memory_addresses(),
                pattern: self.rng.random(),
            },
            2 => Transformation::ConditionalJump {
                condition_regs: self.generate_constrained_register_pair(assigned_registers),
                jump_targets: self.generate_jump_targets(),
            },
            3 => Transformation::EncryptionRound {
                key_addr: self.rng.random_range(0..1024 * 1024) * 4,
                rounds: self.rng.random_range(1..=4) as u8,
            },
            4 => Transformation::RegisterShuffle {
                mapping: self.generate_constrained_shuffle_mapping(assigned_registers),
            },
            _ => unreachable!(),
        }
    }

    /// Generate register set constrained to assigned registers
    fn generate_constrained_register_set(
        &mut self,
        assigned_registers: &[u8],
        max_count: usize,
    ) -> Vec<u8> {
        if assigned_registers.is_empty() {
            return vec![0]; // Fallback to R0 if no registers assigned
        }

        let count = self
            .rng
            .random_range(1..=assigned_registers.len().min(max_count));
        let mut registers = assigned_registers.to_vec();

        // Shuffle and take first `count` registers
        for i in 0..registers.len() {
            let j = self.rng.random_range(i..registers.len());
            registers.swap(i, j);
        }

        registers.truncate(count);
        registers
    }

    /// Generate register pair from assigned registers
    fn generate_constrained_register_pair(&mut self, assigned_registers: &[u8]) -> (u8, u8) {
        if assigned_registers.is_empty() {
            return (0, 1); // Fallback
        }
        if assigned_registers.len() == 1 {
            return (assigned_registers[0], assigned_registers[0]);
        }

        let reg1 = assigned_registers[self.rng.random_range(0..assigned_registers.len())];
        let reg2 = assigned_registers[self.rng.random_range(0..assigned_registers.len())];
        (reg1, reg2)
    }

    /// Generate shuffle mapping constrained to assigned registers
    fn generate_constrained_shuffle_mapping(
        &mut self,
        assigned_registers: &[u8],
    ) -> HashMap<u8, u8> {
        let mut mapping = HashMap::new();

        if assigned_registers.is_empty() {
            mapping.insert(0, 0); // No-op mapping if no registers
            return mapping;
        }

        let mut targets = assigned_registers.to_vec();

        // Shuffle target registers
        for i in 0..targets.len() {
            let j = self.rng.random_range(i..targets.len());
            targets.swap(i, j);
        }

        // Map each assigned register to a shuffled target within the same set
        for (i, &src) in assigned_registers.iter().enumerate() {
            let dst = targets[i % targets.len()];
            mapping.insert(src, dst);
        }

        mapping
    }

    fn generate_arithmetic_ops(&mut self) -> Vec<ArithmeticOp> {
        let count = self.rng.random_range(2..=6);
        (0..count)
            .map(|_| match self.rng.random_range(0..3) {
                0 => ArithmeticOp::Add,
                1 => ArithmeticOp::Sub,
                2 => ArithmeticOp::Xor,
                _ => unreachable!(),
            })
            .collect()
    }

    fn generate_memory_addresses(&mut self) -> Vec<u32> {
        let count = self.rng.random_range(4..=16);
        (0..count)
            .map(|_| {
                // Generate aligned addresses within 1GB space
                self.rng.random_range(0..256 * 1024 * 1024) * 4
            })
            .collect()
    }

    fn generate_jump_targets(&mut self) -> Vec<u32> {
        let count = self.rng.random_range(2..=4);
        (0..count).map(|_| self.rng.random_range(1..=100)).collect()
    }

    fn should_split(&mut self, depth: usize) -> bool {
        let depth_factor = 1.0 - (depth as f32 / self.max_depth as f32);
        self.rng.random::<f32>() < self.split_probability * depth_factor
    }

    fn should_rejoin(&mut self, depth: usize) -> bool {
        let depth_factor = depth as f32 / self.max_depth as f32;
        self.rng.random::<f32>() < self.rejoin_probability * depth_factor
    }
}
