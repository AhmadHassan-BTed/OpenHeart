//! SymbolTableSerializer and SymbolTableArtifact binary serialization (.sta format)
//! Implements 64-byte Header, 64-byte SymbolRecord[], ScopeRecord[], CSR TypeHierarchy, and CRC-64 verification.

use crate::core::io::binary::{BinaryReader, BinaryWriter};
use crate::core::types::symbol::{ScopeRecord, SymbolRecord, UMLAssociationRecord};
use crate::ingestion::serializer::crc64_ecma;
use crate::symbol::builder::{SymbolTableBuilder, TypeHierarchyEdge};
use std::io::{Error, ErrorKind, Result};

pub const STA_MAGIC: u64 = 0x53544100_01000000; // "STA\x00\x01\x00\x00\x00"
pub const STA_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct SymbolTableArtifact {
    pub magic: u64,
    pub format_version: u32,
    pub symbol_count: u32,
    pub scope_count: u32,
    pub th_edge_count: u32,
    pub assoc_count: u32,
    pub qual_name_count: u32,
    pub bpa_hash: u64,
    pub tca_hash: u64,

    pub symbol_records: Vec<SymbolRecord>,
    pub name_index: Vec<(u32, u32)>, // (name_id, symbol_id)
    pub scope_records: Vec<ScopeRecord>,
    pub th_edges: Vec<TypeHierarchyEdge>,
    pub associations: Vec<UMLAssociationRecord>,
    pub qual_names: Vec<String>,
    pub custom_package_names: std::collections::HashMap<u32, String>,
    pub file_package_names: std::collections::HashMap<u16, String>,
    pub crc64_checksum: u64,
}

impl SymbolTableArtifact {
    pub fn symbol(&self, sym_id: u32) -> Option<&SymbolRecord> {
        self.symbol_records.get(sym_id as usize)
    }

    pub fn build(builder: &SymbolTableBuilder, bpa_bytes: &[u8], tca_bytes: &[u8]) -> Self {
        let bpa_hash = crc64_ecma(bpa_bytes);
        let tca_hash = crc64_ecma(tca_bytes);

        let mut name_index: Vec<(u32, u32)> = builder
            .symbols
            .iter()
            .map(|s| (s.name_id, s.symbol_id))
            .collect();

        name_index.sort_by_key(|&(nid, _)| nid);

        let mut artifact = Self {
            magic: STA_MAGIC,
            format_version: STA_FORMAT_VERSION,
            symbol_count: builder.symbols.len() as u32,
            scope_count: builder.scope_graph.scope_count() as u32,
            th_edge_count: builder.th_edges.len() as u32,
            assoc_count: builder.associations.len() as u32,
            qual_name_count: builder.qual_names.len() as u32,
            bpa_hash,
            tca_hash,

            symbol_records: builder.symbols.clone(),
            name_index,
            scope_records: builder.scope_graph.scopes.clone(),
            th_edges: builder.th_edges.clone(),
            associations: builder.associations.clone(),
            qual_names: builder.qual_names.all_names().to_vec(),
            custom_package_names: builder.custom_package_names.clone(),
            file_package_names: builder.file_package_names.clone(),
            crc64_checksum: 0,
        };
        artifact
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buffer = Vec::new();
        {
            let mut writer = BinaryWriter::new(&mut buffer);

            // 1. Header (64 bytes)
            writer.write_u64(self.magic).unwrap();
            writer.write_u32(self.format_version).unwrap();
            writer.write_u32(self.symbol_count).unwrap();
            writer.write_u32(self.scope_count).unwrap();
            writer.write_u32(self.th_edge_count).unwrap();
            writer.write_u32(self.assoc_count).unwrap();
            writer.write_u32(self.qual_name_count).unwrap();
            writer.write_u64(self.bpa_hash).unwrap();
            writer.write_u64(self.tca_hash).unwrap();
            writer.write_bytes(&[0u8; 16]).unwrap(); // _reserved 16B

            // 2. Symbol Table (n_sym * 64B)
            for sym in &self.symbol_records {
                writer.write_u32(sym.symbol_id).unwrap();
                writer.write_u32(sym.name_id).unwrap();
                writer.write_u32(sym.qual_name_id).unwrap();
                writer.write_u32(sym.type_id).unwrap();
                writer.write_u32(sym.decl_node).unwrap();
                writer.write_u32(sym.def_node).unwrap();
                writer.write_u32(sym.parent_sym).unwrap();
                writer.write_u32(sym.first_child).unwrap();
                writer.write_u32(sym.next_sibling).unwrap();
                writer.write_u32(sym.scope_id).unwrap();
                writer.write_u32(sym.uml_meta_offset).unwrap();
                writer.write_u16(sym.param_count).unwrap();
                writer.write_u16(sym.modifiers).unwrap();
                writer.write_u8(sym.kind).unwrap();
                writer.write_u8(sym.visibility).unwrap();
                writer.write_u8(sym.type_param_count).unwrap();
                writer.write_u8(sym.flags).unwrap();
                writer.write_u32(sym.first_token_id).unwrap();
                writer.write_u32(sym.last_token_id).unwrap();
                writer.write_u32(sym._reserved).unwrap();
            }

            // 3. Name Index (n_sym * 8B)
            for &(name_id, sym_id) in &self.name_index {
                writer.write_u32(name_id).unwrap();
                writer.write_u32(sym_id).unwrap();
            }

            // 4. Scope Graph (n_scope * 32B)
            for sc in &self.scope_records {
                writer.write_u32(sc.scope_id).unwrap();
                writer.write_u32(sc.parent_scope).unwrap();
                writer.write_u32(sc.owner_symbol).unwrap();
                writer.write_u32(sc.first_decl).unwrap();
                writer.write_u32(sc.decl_count).unwrap();
                writer.write_u16(sc.import_count).unwrap();
                writer.write_u8(sc.scope_kind).unwrap();
                writer.write_u8(sc.flags).unwrap();
                writer.write_u32(sc.import_table_off).unwrap();
                writer.write_u32(sc._reserved).unwrap();
            }

            // 5. Type Hierarchy Edges
            for edge in &self.th_edges {
                writer.write_u32(edge.from_sym).unwrap();
                writer.write_u32(edge.to_sym).unwrap();
                writer.write_u8(edge.relation as u8).unwrap();
            }

            // 6. Associations (assoc_count * 28B)
            for assoc in &self.associations {
                writer.write_u32(assoc.from_symbol_id).unwrap();
                writer.write_u32(assoc.to_symbol_id).unwrap();
                writer.write_u32(assoc.field_symbol_id).unwrap();
                writer.write_u8(assoc.assoc_kind).unwrap();
                writer.write_u16(assoc.mult_min).unwrap();
                writer.write_u16(assoc.mult_max).unwrap();
                writer.write_u8(assoc.is_navigable).unwrap();
                writer.write_u32(assoc.role_name_id).unwrap();
                writer.write_u32(assoc._reserved).unwrap();
                writer.write_u16(assoc._padding).unwrap();
            }

            // 7. Qual Names
            for name in &self.qual_names {
                writer.write_u32(name.len() as u32).unwrap();
                writer.write_bytes(name.as_bytes()).unwrap();
            }

            // 8. Custom Package Names Map
            writer
                .write_u32(self.custom_package_names.len() as u32)
                .unwrap();
            for (sym_id, name) in &self.custom_package_names {
                writer.write_u32(*sym_id).unwrap();
                let name_bytes = name.as_bytes();
                writer.write_u32(name_bytes.len() as u32).unwrap();
                writer.write_bytes(name_bytes).unwrap();
            }

            // 9. File Package Names Map
            writer
                .write_u32(self.file_package_names.len() as u32)
                .unwrap();
            for (fid, name) in &self.file_package_names {
                writer.write_u16(*fid).unwrap();
                let name_bytes = name.as_bytes();
                writer.write_u32(name_bytes.len() as u32).unwrap();
                writer.write_bytes(name_bytes).unwrap();
            }
        }

        // 8. CRC-64 Checksum over all payload bytes
        let checksum = crc64_ecma(&buffer);
        let mut final_buffer = buffer;
        let mut writer = BinaryWriter::new(&mut final_buffer);
        writer.write_u64(checksum).unwrap();

        final_buffer
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 72 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "File too small for STA artifact",
            ));
        }

        let payload_len = bytes.len() - 8;
        let payload = &bytes[..payload_len];
        let mut crc_reader = BinaryReader::new(&bytes[payload_len..]);
        let expected_checksum = crc_reader.read_u64()?;
        let actual_checksum = crc64_ecma(payload);

        if expected_checksum != actual_checksum {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "STA checksum mismatch: expected {:#x}, got {:#x}",
                    expected_checksum, actual_checksum
                ),
            ));
        }

        let mut reader = BinaryReader::new(payload);
        let magic = reader.read_u64()?;
        if magic != STA_MAGIC {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("Invalid STA magic: {:#x}", magic),
            ));
        }

        let format_version = reader.read_u32()?;
        let symbol_count = reader.read_u32()?;
        let scope_count = reader.read_u32()?;
        let th_edge_count = reader.read_u32()?;
        let assoc_count = reader.read_u32()?;
        let qual_name_count = reader.read_u32()?;
        let bpa_hash = reader.read_u64()?;
        let tca_hash = reader.read_u64()?;
        let _reserved = reader.read_exact_bytes(16)?;

        let mut symbol_records = Vec::with_capacity(symbol_count as usize);
        for _ in 0..symbol_count {
            let symbol_id = reader.read_u32()?;
            let name_id = reader.read_u32()?;
            let qual_name_id = reader.read_u32()?;
            let type_id = reader.read_u32()?;
            let decl_node = reader.read_u32()?;
            let def_node = reader.read_u32()?;
            let parent_sym = reader.read_u32()?;
            let first_child = reader.read_u32()?;
            let next_sibling = reader.read_u32()?;
            let scope_id = reader.read_u32()?;
            let uml_meta_offset = reader.read_u32()?;
            let param_count = reader.read_u16()?;
            let modifiers = reader.read_u16()?;
            let kind = reader.read_u8()?;
            let visibility = reader.read_u8()?;
            let type_param_count = reader.read_u8()?;
            let flags = reader.read_u8()?;
            let first_token_id = reader.read_u32()?;
            let last_token_id = reader.read_u32()?;
            let _reserved = reader.read_u32()?;

            symbol_records.push(SymbolRecord {
                symbol_id,
                name_id,
                qual_name_id,
                type_id,
                decl_node,
                def_node,
                parent_sym,
                first_child,
                next_sibling,
                scope_id,
                uml_meta_offset,
                param_count,
                modifiers,
                kind,
                visibility,
                type_param_count,
                flags,
                first_token_id,
                last_token_id,
                _reserved,
            });
        }

        let mut name_index = Vec::with_capacity(symbol_count as usize);
        for _ in 0..symbol_count {
            let name_id = reader.read_u32()?;
            let sym_id = reader.read_u32()?;
            name_index.push((name_id, sym_id));
        }

        let mut scope_records = Vec::with_capacity(scope_count as usize);
        for _ in 0..scope_count {
            let scope_id = reader.read_u32()?;
            let parent_scope = reader.read_u32()?;
            let owner_symbol = reader.read_u32()?;
            let first_decl = reader.read_u32()?;
            let decl_count = reader.read_u32()?;
            let import_count = reader.read_u16()?;
            let scope_kind = reader.read_u8()?;
            let flags = reader.read_u8()?;
            let import_table_off = reader.read_u32()?;
            let _reserved = reader.read_u32()?;

            scope_records.push(ScopeRecord {
                scope_id,
                parent_scope,
                owner_symbol,
                first_decl,
                decl_count,
                import_count,
                scope_kind,
                flags,
                import_table_off,
                _reserved,
            });
        }

        let mut th_edges = Vec::with_capacity(th_edge_count as usize);
        for _ in 0..th_edge_count {
            let from_sym = reader.read_u32()?;
            let to_sym = reader.read_u32()?;
            let relation = reader.read_u8()?.into();
            th_edges.push(TypeHierarchyEdge {
                from_sym,
                to_sym,
                relation,
            });
        }

        let mut associations = Vec::with_capacity(assoc_count as usize);
        for _ in 0..assoc_count {
            let from_symbol_id = reader.read_u32()?;
            let to_symbol_id = reader.read_u32()?;
            let field_symbol_id = reader.read_u32()?;
            let assoc_kind = reader.read_u8()?;
            let mult_min = reader.read_u16()?;
            let mult_max = reader.read_u16()?;
            let is_navigable = reader.read_u8()?;
            let role_name_id = reader.read_u32()?;
            let _reserved = reader.read_u32()?;
            let _padding = reader.read_u16()?;

            associations.push(UMLAssociationRecord {
                from_symbol_id,
                to_symbol_id,
                field_symbol_id,
                assoc_kind,
                mult_min,
                mult_max,
                is_navigable,
                role_name_id,
                _reserved,
                _padding,
            });
        }

        let mut qual_names = Vec::with_capacity(qual_name_count as usize);
        for _ in 0..qual_name_count {
            let len = reader.read_u32()? as usize;
            let bytes = reader.read_exact_bytes(len)?;
            let str_val = String::from_utf8(bytes).map_err(|e| {
                Error::new(
                    ErrorKind::InvalidData,
                    format!("Invalid UTF-8 in STA: {}", e),
                )
            })?;
            qual_names.push(str_val);
        }

        let mut custom_package_names = std::collections::HashMap::new();
        if let Ok(custom_count) = reader.read_u32() {
            for _ in 0..custom_count {
                if let (Ok(sym_id), Ok(len)) = (reader.read_u32(), reader.read_u32()) {
                    if let Ok(bytes) = reader.read_exact_bytes(len as usize) {
                        if let Ok(name) = String::from_utf8(bytes) {
                            custom_package_names.insert(sym_id, name);
                        }
                    }
                }
            }
        }

        let mut file_package_names = std::collections::HashMap::new();
        if let Ok(file_count) = reader.read_u32() {
            for _ in 0..file_count {
                if let (Ok(fid), Ok(len)) = (reader.read_u16(), reader.read_u32()) {
                    if let Ok(bytes) = reader.read_exact_bytes(len as usize) {
                        if let Ok(name) = String::from_utf8(bytes) {
                            file_package_names.insert(fid, name);
                        }
                    }
                }
            }
        }

        Ok(Self {
            magic,
            format_version,
            symbol_count,
            scope_count,
            th_edge_count,
            assoc_count,
            qual_name_count,
            bpa_hash,
            tca_hash,
            symbol_records,
            name_index,
            scope_records,
            th_edges,
            associations,
            qual_names,
            custom_package_names,
            file_package_names,
            crc64_checksum: expected_checksum,
        })
    }
}
