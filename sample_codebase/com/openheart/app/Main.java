package com.openheart.app;

import com.openheart.app.service.OrderService;
import com.openheart.app.pattern.SingletonConfig;
import com.openheart.app.pattern.OrderFactory;
import com.openheart.app.model.Order;

public class Main {
    public static void main(String[] args) {
        SingletonConfig config = SingletonConfig.getInstance();
        OrderService service = new OrderService();
        Order order = OrderFactory.createOrder(101, "CustomerA", 250.50);
        service.processOrder(order);
        System.out.println("Processing completed for order: " + order.getId());
    }
}
