package com.enterprise.bank.app;

import com.enterprise.bank.config.DatabaseConfig;
import com.enterprise.bank.core.AccountStatus;
import com.enterprise.bank.core.TransactionType;
import com.enterprise.bank.model.SavingsAccount;
import com.enterprise.bank.model.CheckingAccount;
import com.enterprise.bank.model.LedgerTransaction;
import com.enterprise.bank.service.TransferService;

public class MainApplication {
    public static void main(String[] args) {
        DatabaseConfig config = DatabaseConfig.getInstance();
        System.out.println("Database URL: " + config.getConnectionUrl());

        SavingsAccount savings = new SavingsAccount(101, "Alice", 5000.0, 0.05);
        CheckingAccount checking = new CheckingAccount(102, "Bob", 1200.0, 500.0);

        savings.setStatus(AccountStatus.ACTIVE);
        checking.setStatus(AccountStatus.ACTIVE);

        savings.applyInterest();

        LedgerTransaction tx = new LedgerTransaction(901, savings.getId(), checking.getId(), 450.0, TransactionType.TRANSFER);
        TransferService transferService = new TransferService();

        boolean result = transferService.executeTransfer(savings, checking, tx);
        System.out.println("Transfer Success: " + result);
        System.out.println("Savings Balance: " + savings.getBalance() + " | Checking Balance: " + checking.getBalance());
    }
}
