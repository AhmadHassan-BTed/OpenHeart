package com.enterprise.system.model;

import com.enterprise.system.core.BaseEntity;

public class Transaction extends BaseEntity {
    private long sourceAccountId;
    private long targetAccountId;
    private double amount;

    public Transaction(long id, long sourceAccountId, long targetAccountId, double amount) {
        super(id);
        this.sourceAccountId = sourceAccountId;
        this.targetAccountId = targetAccountId;
        this.amount = amount;
    }

    public long getSourceAccountId() {
        return sourceAccountId;
    }

    public long getTargetAccountId() {
        return targetAccountId;
    }

    public double getAmount() {
        return amount;
    }
}
