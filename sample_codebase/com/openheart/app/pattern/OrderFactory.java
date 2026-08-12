package com.openheart.app.pattern;

import com.openheart.app.model.Order;

public class OrderFactory {
    public static Order createOrder(int id, String customer, double amount) {
        return new Order(id, customer, amount);
    }
}
