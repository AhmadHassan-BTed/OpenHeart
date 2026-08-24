package com.enterprise.bank.model;

public class CheckingAccount extends UserAccount {
    private double overdraftLimit;

    public CheckingAccount(long id, String holderName, double initialBalance, double overdraftLimit) {
        super(id, holderName, initialBalance);
        this.overdraftLimit = overdraftLimit;
    }

    public double getOverdraftLimit() {
        return overdraftLimit;
    }

    @Override
    public boolean withdraw(double amount) {
        if (amount > 0 && (getBalance() + overdraftLimit) >= amount) {
            deposit(-amount);
            return true;
        }
        return false;
    }
}
