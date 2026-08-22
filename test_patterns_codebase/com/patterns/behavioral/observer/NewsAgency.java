package com.patterns.behavioral.observer;

import java.util.ArrayList;
import java.util.List;

public class NewsAgency implements Subject {
    private String news;
    private List<Observer> observers = new ArrayList<>();

    public void setNews(String news) {
        this.news = news;
        notifyObservers();
    }

    @Override
    public void attach(Observer observer) {
        this.observers.add(observer);
    }

    @Override
    public void detach(Observer observer) {
        this.observers.remove(observer);
    }

    @Override
    public void notifyObservers() {
        for (Observer observer : this.observers) {
            observer.update(this.news);
        }
    }
}
