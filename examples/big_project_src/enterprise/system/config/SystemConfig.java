package com.enterprise.system.config;

public class SystemConfig {
    private static SystemConfig instance;
    private String environment;
    private int maxConnections;

    private SystemConfig() {
        this.environment = "PRODUCTION";
        this.maxConnections = 100;
    }

    public static synchronized SystemConfig getInstance() {
        if (instance == null) {
            instance = new SystemConfig();
        }
        return instance;
    }

    public String getEnvironment() {
        return environment;
    }

    public int getMaxConnections() {
        return maxConnections;
    }
}
