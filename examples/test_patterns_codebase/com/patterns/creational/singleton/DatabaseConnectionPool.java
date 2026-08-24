package com.patterns.creational.singleton;

public class DatabaseConnectionPool {
    private static DatabaseConnectionPool instance;
    private int maxConnections;
    private boolean isInitialized;

    private DatabaseConnectionPool() {
        this.maxConnections = 10;
        this.isInitialized = true;
    }

    public static DatabaseConnectionPool getInstance() {
        if (instance == null) {
            instance = new DatabaseConnectionPool();
        }
        return instance;
    }

    public void executeQuery(String sql) {
        System.out.println("Executing: " + sql);
    }
}
