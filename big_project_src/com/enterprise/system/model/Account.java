package com.enterprise.system.model;

import com.enterprise.system.core.BaseEntity;

public class Account extends BaseEntity {
    private long userId;
    private double balance;

    public Account(long id, long userId, double initialBalance) {
        super(id);
        this.userId = userId;
        this.balance = initialBalance;
    }

    public long getUserId() {
        return userId;
    }

    public double getBalance() {
        return balance;
    }

    public void deposit(double amount) {
        if (amount > 0) {
            this.balance += amount;
        }
    }

    public boolean withdraw(double amount) {
        if (amount > 0 && this.balance >= amount) {
            this.balance -= amount;
            return true;
        }
        return false;
    }
}
