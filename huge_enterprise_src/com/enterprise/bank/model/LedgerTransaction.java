package com.enterprise.bank.model;

import com.enterprise.bank.core.BaseModel;
import com.enterprise.bank.core.TransactionType;

public class LedgerTransaction extends BaseModel {
    private long sourceId;
    private long targetId;
    private double amount;
    private TransactionType type;

    public LedgerTransaction(long id, long sourceId, long targetId, double amount, TransactionType type) {
        super(id);
        this.sourceId = sourceId;
        this.targetId = targetId;
        this.amount = amount;
        this.type = type;
    }

    public long getSourceId() {
        return sourceId;
    }

    public long getTargetId() {
        return targetId;
    }

    public double getAmount() {
        return amount;
    }

    public TransactionType getType() {
        return type;
    }
}
