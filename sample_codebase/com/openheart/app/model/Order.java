package com.openheart.app.model;

public class Order {
    private int id;
    private String customer;
    private double amount;
    private String state;

    public Order(int id, String customer, double amount) {
        this.id = id;
        this.customer = customer;
        this.amount = amount;
        this.state = "CREATED";
    }

    public int getId() {
        return id;
    }

    public String getCustomer() {
        return customer;
    }

    public double getAmount() {
        return amount;
    }

    public String getState() {
        return state;
    }

    public void setState(String state) {
        this.state = state;
    }
}
