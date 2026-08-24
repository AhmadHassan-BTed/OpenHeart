package com.enterprise.bank.model;

public class SavingsAccount extends UserAccount {
    private double interestRate;

    public SavingsAccount(long id, String holderName, double initialBalance, double interestRate) {
        super(id, holderName, initialBalance);
        this.interestRate = interestRate;
    }

    public double getInterestRate() {
        return interestRate;
    }

    public void applyInterest() {
        double interest = getBalance() * interestRate;
        deposit(interest);
    }
}
