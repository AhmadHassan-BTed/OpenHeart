package com.enterprise.system.core;

public record AuditRecord(long entityId, String action, long timestamp) {}
