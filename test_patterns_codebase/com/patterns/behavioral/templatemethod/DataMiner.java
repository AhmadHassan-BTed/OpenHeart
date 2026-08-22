package com.patterns.behavioral.templatemethod;

public abstract class DataMiner {
    public final void mine(String path) {
        openFile(path);
        extractData();
        parseData();
        closeFile();
    }

    public abstract void openFile(String path);
    public abstract void extractData();
    public abstract void parseData();

    public void closeFile() {
        System.out.println("File closed.");
    }
}
