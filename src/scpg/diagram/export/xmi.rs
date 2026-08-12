//! XMI Exporter — exports UML diagrams to Eclipse-compatible XMI / UML 2.x XML format (§10.4).

use crate::uma::types::*;

pub struct XMIExporter;

impl XMIExporter {
    pub fn export_class_diagram(classes: &[ClassRecord]) -> String {
        let mut xmi = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xmi.push_str("<uml:Model xmi:version=\"2.1\" xmlns:xmi=\"http://schema.omg.org/spec/XMI/2.1\" xmlns:uml=\"http://www.eclipse.org/uml2/3.0.0/UML\" name=\"OpenHeartSCPG\">\n");
        for class_rec in classes {
            xmi.push_str(&format!(
                "  <packagedElement xmi:type=\"uml:Class\" xmi:id=\"sym_{}\" name=\"Class_{}\"/>\n",
                class_rec.sym_id, class_rec.sym_id
            ));
        }
        xmi.push_str("</uml:Model>\n");
        xmi
    }
}
