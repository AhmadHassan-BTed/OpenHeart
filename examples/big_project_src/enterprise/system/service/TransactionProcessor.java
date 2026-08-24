package com.enterprise.system.service;

import com.enterprise.system.core.StatusEnum;
import com.enterprise.system.model.Account;
import com.enterprise.system.model.Transaction;

public class TransactionProcessor {
    public boolean processTransaction(Account source, Account target, Transaction tx) {
        if (source == null || target == null || tx == null) {
            return false;
        }

        if (source.getStatus() != StatusEnum.ACTIVE || target.getStatus() != StatusEnum.ACTIVE) {
            tx.setStatus(StatusEnum.ERROR);
            return false;
        }

        if (tx.getAmount() <= 0) {
            tx.setStatus(StatusEnum.ERROR);
            return false;
        }

        if (source.withdraw(tx.getAmount())) {
            target.deposit(tx.getAmount());
            tx.setStatus(StatusEnum.CLOSED);
            return true;
        } else {
            tx.setStatus(StatusEnum.SUSPENDED);
            return false;
        }
    }
}
