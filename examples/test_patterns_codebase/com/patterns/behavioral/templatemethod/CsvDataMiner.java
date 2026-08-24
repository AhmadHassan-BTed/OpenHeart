package com.patterns.behavioral.templatemethod;

public class CsvDataMiner extends DataMiner {
    @Override
    public void openFile(String path) {
        System.out.println("Opening CSV: " + path);
    }

    @Override
    public void extractData() {
        System.out.println("Extracting CSV lines.");
    }

    @Override
    public void parseData() {
        System.out.println("Parsing CSV comma records.");
    }
}
