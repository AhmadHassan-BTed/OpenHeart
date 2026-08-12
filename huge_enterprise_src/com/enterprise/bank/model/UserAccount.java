package com.enterprise.bank.model;

import com.enterprise.bank.core.BaseModel;
import com.enterprise.bank.core.AccountStatus;

public class UserAccount extends BaseModel {
    private String holderName;
    private double balance;
    private AccountStatus status;

    public UserAccount(long id, String holderName, double initialBalance) {
        super(id);
        this.holderName = holderName;
        this.balance = initialBalance;
        this.status = AccountStatus.UNVERIFIED;
    }

    public String getHolderName() {
        return holderName;
    }

    public double getBalance() {
        return balance;
    }

    public AccountStatus getStatus() {
        return status;
    }

    public void setStatus(AccountStatus status) {
        this.status = status;
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
