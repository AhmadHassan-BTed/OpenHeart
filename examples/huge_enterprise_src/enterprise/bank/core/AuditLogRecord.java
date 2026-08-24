package com.enterprise.bank.core;

public record AuditLogRecord(long eventId, String details, long timestamp) {}
