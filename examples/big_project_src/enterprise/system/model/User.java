package com.enterprise.system.model;

import com.enterprise.system.core.BaseEntity;

public class User extends BaseEntity {
    private String username;
    private String email;

    public User(long id, String username, String email) {
        super(id);
        this.username = username;
        this.email = email;
    }

    public String getUsername() {
        return username;
    }

    public String getEmail() {
        return email;
    }
}
