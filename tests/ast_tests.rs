use tempfile::tempdir;

use openheart::ast::adapter::java::JavaASTReductionAdapter;
use openheart::ast::adapter::ASTReductionAdapter;
use openheart::ast::bp_encoder::BPEncoder;
use openheart::ast::builder::BPASTBuilder;
use openheart::ast::jump_table::JumpTableBuilder;
use openheart::ast::rank_select::RankSelectIndex;
use openheart::ast::reducer::reduce_and_encode;
use openheart::ast::rmq::SparseTableRMQ;
use openheart::ast::serializer::BPASTSerializer;
use openheart::ingestion::parser::tree_sitter::TreeSitterParser;
use openheart::ingestion::parser::CSTParser;

#[test]
fn test_bp_encoder_basic() {
    let mut bp = BPEncoder::new();
    bp.push_open(); // bit 0: 1
    bp.push_open(); // bit 1: 1
    bp.push_close(); // bit 2: 0
    bp.push_close(); // bit 3: 0

    assert_eq!(bp.len(), 4);
    assert_eq!(bp.get_bit(0), 1);
    assert_eq!(bp.get_bit(1), 1);
    assert_eq!(bp.get_bit(2), 0);
    assert_eq!(bp.get_bit(3), 0);
}

#[test]
fn test_jump_table_and_rank_select() {
    let mut bp = BPEncoder::new();
    // Tree: Root( Child1, Child2 )
    // BP: 1 1 0 1 0 0
    bp.push_open(); // 0: Root open
    bp.push_open(); // 1: C1 open
    bp.push_close(); // 2: C1 close
    bp.push_open(); // 3: C2 open
    bp.push_close(); // 4: C2 close
    bp.push_close(); // 5: Root close

    let jump = JumpTableBuilder::build(&bp);
    assert_eq!(jump.match_pos[0], 5);
    assert_eq!(jump.match_pos[5], 0);
    assert_eq!(jump.match_pos[1], 2);
    assert_eq!(jump.match_pos[2], 1);
    assert_eq!(jump.match_pos[3], 4);
    assert_eq!(jump.match_pos[4], 3);

    let rs = RankSelectIndex::build(&bp);
    assert_eq!(rs.rank1(&bp, 0), 1);
    assert_eq!(rs.rank1(&bp, 1), 2);
    assert_eq!(rs.rank1(&bp, 2), 2);
    assert_eq!(rs.rank1(&bp, 3), 3);
    assert_eq!(rs.rank1(&bp, 5), 3);

    assert_eq!(rs.select1(&bp, 1), 0);
    assert_eq!(rs.select1(&bp, 2), 1);
    assert_eq!(rs.select1(&bp, 3), 3);
}

#[test]
fn test_sparse_table_rmq_lca() {
    let mut bp = BPEncoder::new();
    // Root( C1, C2 ) -> BP: 1 1 0 1 0 0
    bp.push_open(); // 0
    bp.push_open(); // 1
    bp.push_close(); // 2
    bp.push_open(); // 3
    bp.push_close(); // 4
    bp.push_close(); // 5

    let rs = RankSelectIndex::build(&bp);
    let rmq = SparseTableRMQ::build(&bp, &rs);

    // LCA of node 1 (C1) and node 2 (C2) -> node 0 (Root)
    let lca = rmq.lca(&bp, &rs, 1, 2);
    assert_eq!(lca, 0);
}

#[test]
fn test_java_cst_reduction_end_to_end() {
    let sample_code = r#"
        public class HelloWorld {
            public static void main(String[] args) {
                int x = 42;
                if (x > 0) {
                    System.out.println("Hello");
                }
            }
        }
    "#;

    let adapter = JavaASTReductionAdapter::new();
    let mut parser = TreeSitterParser::new().unwrap();
    let tree = parser
        .parse(sample_code.as_bytes(), adapter.ts_language())
        .unwrap();

    let mut builder = BPASTBuilder::new(100, 0x123456789ABCDEF0);
    reduce_and_encode(
        tree.root_node(),
        sample_code.as_bytes(),
        1,
        &adapter,
        &[],
        &mut builder,
    );

    let artifact = builder.finalize();

    // Verify Phase 2 Invariants 1-5
    assert!(artifact.node_count > 0);
    assert_eq!(
        artifact.bp_encoder.bit_count,
        (artifact.node_count * 2) as usize
    );

    // Invariant 1: BP Balance
    assert_eq!(
        artifact
            .rank_select
            .rank1(&artifact.bp_encoder, artifact.bp_encoder.bit_count - 1),
        artifact.node_count
    );

    // Invariant 2: Jump Table Inverse Match
    for i in 0..artifact.bp_encoder.bit_count {
        let m = artifact.jump_table.match_pos[i] as usize;
        assert_eq!(artifact.jump_table.match_pos[m] as usize, i);
    }

    // Invariant 4: Parent Map Ordering
    for i in 1..artifact.node_count as usize {
        let parent = artifact.preorder.parent_map[i];
        assert!(parent < i as u32);
    }

    // Invariant 5: TCA Hash Link
    assert_eq!(artifact.tca_hash, 0x123456789ABCDEF0);

    // Test Roundtrip Serialization & Deserialization
    let dir = tempdir().unwrap();
    let bpa_path = dir.path().join("test.bpa");
    BPASTSerializer::write(&artifact, &bpa_path).unwrap();

    let loaded_artifact = BPASTSerializer::read(&bpa_path).unwrap();
    assert_eq!(loaded_artifact.node_count, artifact.node_count);
    assert_eq!(loaded_artifact.bp_encoder.words, artifact.bp_encoder.words);
    assert_eq!(
        loaded_artifact.preorder.node_types,
        artifact.preorder.node_types
    );
    assert_eq!(loaded_artifact.tca_hash, artifact.tca_hash);
}
