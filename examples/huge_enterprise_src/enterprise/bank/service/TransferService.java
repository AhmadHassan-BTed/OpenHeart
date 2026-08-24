package com.enterprise.bank.service;

import com.enterprise.bank.core.AccountStatus;
import com.enterprise.bank.model.UserAccount;
import com.enterprise.bank.model.LedgerTransaction;

public class TransferService {
    public boolean executeTransfer(UserAccount source, UserAccount target, LedgerTransaction tx) {
        if (source == null || target == null || tx == null) {
            return false;
        }

        if (source.getStatus() != AccountStatus.ACTIVE || target.getStatus() != AccountStatus.ACTIVE) {
            return false;
        }

        if (tx.getAmount() <= 0) {
            return false;
        }

        if (source.withdraw(tx.getAmount())) {
            target.deposit(tx.getAmount());
            return true;
        }
        return false;
    }
}
