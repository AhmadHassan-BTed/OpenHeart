package com.enterprise.system.app;

import com.enterprise.system.config.SystemConfig;
import com.enterprise.system.core.StatusEnum;

import com.enterprise.system.model.Account;
import com.enterprise.system.model.Transaction;
import com.enterprise.system.model.User;
import com.enterprise.system.service.TransactionProcessor;

public class Application {
    public static void main(String[] args) {
        SystemConfig config = SystemConfig.getInstance();
        System.out.println("Starting system in environment: " + config.getEnvironment());

        User user1 = new User(1001, "Alice", "alice@enterprise.com");
        User user2 = new User(1002, "Bob", "bob@enterprise.com");
        user1.setStatus(StatusEnum.ACTIVE);
        user2.setStatus(StatusEnum.ACTIVE);

        Account acc1 = new Account(5001, user1.getId(), 1000.0);
        Account acc2 = new Account(5002, user2.getId(), 500.0);
        acc1.setStatus(StatusEnum.ACTIVE);
        acc2.setStatus(StatusEnum.ACTIVE);

        Transaction tx = new Transaction(9001, acc1.getId(), acc2.getId(), 350.0);
        TransactionProcessor processor = new TransactionProcessor();

        boolean success = processor.processTransaction(acc1, acc2, tx);
        System.out.println("Transaction result: " + success + " | Tx Status: " + tx.getStatus());
        System.out.println("Acc1 Balance: " + acc1.getBalance() + " | Acc2 Balance: " + acc2.getBalance());
    }
}
