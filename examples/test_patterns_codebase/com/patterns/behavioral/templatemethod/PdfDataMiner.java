package com.patterns.behavioral.templatemethod;

public class PdfDataMiner extends DataMiner {
    @Override
    public void openFile(String path) {
        System.out.println("Opening PDF: " + path);
    }

    @Override
    public void extractData() {
        System.out.println("Extracting raw PDF bytes.");
    }

    @Override
    public void parseData() {
        System.out.println("Parsing PDF structure.");
    }
}
