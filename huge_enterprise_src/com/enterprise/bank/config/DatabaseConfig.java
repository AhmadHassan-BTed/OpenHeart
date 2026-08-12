package com.enterprise.bank.config;

public class DatabaseConfig {
    private static DatabaseConfig instance;
    private String connectionUrl;
    private int poolSize;

    private DatabaseConfig() {
        this.connectionUrl = "jdbc:postgresql://localhost:5432/enterprise_bank";
        this.poolSize = 50;
    }

    public static synchronized DatabaseConfig getInstance() {
        if (instance == null) {
            instance = new DatabaseConfig();
        }
        return instance;
    }

    public String getConnectionUrl() {
        return connectionUrl;
    }

    public int getPoolSize() {
        return poolSize;
    }
}
