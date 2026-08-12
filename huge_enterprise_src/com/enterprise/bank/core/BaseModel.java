package com.enterprise.bank.core;

public abstract class BaseModel implements Entity {
    private long id;
    private long createdAt;

    public BaseModel(long id) {
        this.id = id;
        this.createdAt = System.currentTimeMillis();
    }

    @Override
    public long getId() {
        return id;
    }

    @Override
    public long getCreatedAt() {
        return createdAt;
    }
}
