package com.enterprise.system.core;

public abstract class BaseEntity implements Identifiable {
    private long id;
    private StatusEnum status;
    private long createdAt;

    public BaseEntity(long id) {
        this.id = id;
        this.status = StatusEnum.PENDING;
        this.createdAt = System.currentTimeMillis();
    }

    @Override
    public long getId() {
        return id;
    }

    public StatusEnum getStatus() {
        return status;
    }

    public void setStatus(StatusEnum status) {
        this.status = status;
    }

    public long getCreatedAt() {
        return createdAt;
    }
}
