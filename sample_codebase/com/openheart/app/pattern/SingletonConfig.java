package com.openheart.app.pattern;

public class SingletonConfig {
    private static SingletonConfig instance;
    private String environment;

    private SingletonConfig() {
        this.environment = "PRODUCTION";
    }

    public static synchronized SingletonConfig getInstance() {
        if (instance == null) {
            instance = new SingletonConfig();
        }
        return instance;
    }

    public String getEnvironment() {
        return environment;
    }
}
