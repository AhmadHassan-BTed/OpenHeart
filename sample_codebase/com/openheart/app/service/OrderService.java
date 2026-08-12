package com.openheart.app.service;

import com.openheart.app.model.Order;

public class OrderService {
    public void processOrder(Order order) {
        if (order == null) {
            return;
        }
        if (order.getAmount() > 100.0) {
            order.setState("APPROVED");
            notifyCustomer(order);
        } else {
            order.setState("PENDING");
        }
    }

    private void notifyCustomer(Order order) {
        System.out.println("Notification sent for order: " + order.getId());
    }
}
