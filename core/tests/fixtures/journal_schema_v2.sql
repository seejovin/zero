-- Schema from released 0.8.0 core, fc17e07fc2726db1b7c37fd45798141cef543305.

CREATE TABLE metadata (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;
CREATE TABLE payments (
    intent_id TEXT PRIMARY KEY NOT NULL,
    chain_id INTEGER NOT NULL CHECK (chain_id IN (1, 8453)),
    sender TEXT NOT NULL,
    nonce TEXT NOT NULL,
    active INTEGER NOT NULL CHECK (active IN (0, 1)),
    prepared_json TEXT NOT NULL,
    signed_json TEXT,
    record_json TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX payment_nonce_reservation ON payments(chain_id, sender, nonce) WHERE active = 1;
CREATE TABLE payment_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    intent_id TEXT NOT NULL REFERENCES payments(intent_id),
    kind TEXT NOT NULL,
    observed_at INTEGER NOT NULL,
    payload_json TEXT NOT NULL
) STRICT;
CREATE INDEX payment_event_intent ON payment_events(intent_id, sequence);
CREATE TABLE balance_snapshots (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    block_hash TEXT NOT NULL UNIQUE,
    observed_at INTEGER NOT NULL,
    snapshot_json TEXT NOT NULL
) STRICT;
CREATE TRIGGER immutable_payment_fields BEFORE UPDATE ON payments WHEN
    NEW.intent_id <> OLD.intent_id OR NEW.chain_id <> OLD.chain_id OR NEW.sender <> OLD.sender
    OR NEW.nonce <> OLD.nonce OR NEW.prepared_json <> OLD.prepared_json
    OR (OLD.signed_json IS NOT NULL AND NEW.signed_json IS NOT OLD.signed_json)
    OR (OLD.active = 0 AND NEW.active <> 0)
BEGIN SELECT RAISE(ABORT, 'immutable payment evidence'); END;
CREATE TRIGGER retain_payments BEFORE DELETE ON payments
BEGIN SELECT RAISE(ABORT, 'payment deletion is prohibited'); END;
CREATE TRIGGER retain_event_updates BEFORE UPDATE ON payment_events
BEGIN SELECT RAISE(ABORT, 'event updates are prohibited'); END;
CREATE TRIGGER retain_event_deletes BEFORE DELETE ON payment_events
BEGIN SELECT RAISE(ABORT, 'event deletion is prohibited'); END;
CREATE TRIGGER retain_balance_updates BEFORE UPDATE ON balance_snapshots
BEGIN SELECT RAISE(ABORT, 'balance observation updates are prohibited'); END;
CREATE TRIGGER retain_balance_deletes BEFORE DELETE ON balance_snapshots
BEGIN SELECT RAISE(ABORT, 'balance observation deletion is prohibited'); END;
CREATE TRIGGER immutable_metadata_updates BEFORE UPDATE ON metadata
BEGIN SELECT RAISE(ABORT, 'journal identity is immutable'); END;
CREATE TRIGGER immutable_metadata_deletes BEFORE DELETE ON metadata
BEGIN SELECT RAISE(ABORT, 'journal identity is immutable'); END;

PRAGMA user_version = 2;
PRAGMA application_id = 1280786764;
