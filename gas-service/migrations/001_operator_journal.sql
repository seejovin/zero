CREATE TABLE metadata (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    policy_json TEXT NOT NULL,
    policy_digest TEXT NOT NULL
) STRICT;

CREATE TABLE treasury_anchor (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    body TEXT NOT NULL
) STRICT;

CREATE TABLE fee_publisher (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    public_key TEXT NOT NULL
) STRICT;

CREATE TABLE fee_schedules (
    schedule_id TEXT PRIMARY KEY,
    policy_json TEXT NOT NULL,
    policy_digest TEXT NOT NULL
) STRICT;

CREATE TABLE fee_schedule_packets (
    schedule_id TEXT PRIMARY KEY REFERENCES fee_schedules(schedule_id),
    packet_json TEXT NOT NULL
) STRICT;

CREATE TABLE jobs (
    request_id TEXT PRIMARY KEY,
    operation_digest TEXT NOT NULL UNIQUE,
    envelope_json TEXT NOT NULL,
    verified_operation_json TEXT NOT NULL,
    admitted_at INTEGER NOT NULL CHECK (admitted_at > 0)
) STRICT;

-- The database is bound to one chain/pool/operator by immutable metadata.
-- Released proofs do not lose their nullifier reservation on quote expiry,
-- timeout, failure, or a local claim that they were cancelled.
CREATE TABLE nullifier_claims (
    tree_number INTEGER NOT NULL CHECK (tree_number >= 0 AND tree_number <= 65535),
    nullifier TEXT NOT NULL,
    request_id TEXT NOT NULL REFERENCES jobs(request_id),
    PRIMARY KEY (tree_number, nullifier)
) STRICT;

CREATE TABLE outer_nonce_claims (
    nonce TEXT PRIMARY KEY,
    request_id TEXT NOT NULL REFERENCES jobs(request_id),
    attempt INTEGER NOT NULL CHECK (attempt >= 1 AND attempt <= 3),
    UNIQUE (request_id, attempt)
) STRICT;

CREATE TABLE events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT NOT NULL REFERENCES jobs(request_id),
    body TEXT NOT NULL
) STRICT;
CREATE INDEX events_by_job ON events(request_id, seq DESC);

CREATE TABLE incidents (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT NOT NULL REFERENCES jobs(request_id),
    code TEXT NOT NULL
) STRICT;

CREATE TRIGGER metadata_no_update BEFORE UPDATE ON metadata BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER metadata_no_delete BEFORE DELETE ON metadata BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER treasury_anchor_no_update BEFORE UPDATE ON treasury_anchor BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER treasury_anchor_no_delete BEFORE DELETE ON treasury_anchor BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER fee_publisher_no_update BEFORE UPDATE ON fee_publisher BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER fee_publisher_no_delete BEFORE DELETE ON fee_publisher BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER fee_schedules_no_update BEFORE UPDATE ON fee_schedules BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER fee_schedules_no_delete BEFORE DELETE ON fee_schedules BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER fee_schedule_packets_no_update BEFORE UPDATE ON fee_schedule_packets BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER fee_schedule_packets_no_delete BEFORE DELETE ON fee_schedule_packets BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER jobs_no_update BEFORE UPDATE ON jobs BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER jobs_no_delete BEFORE DELETE ON jobs BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER nullifier_claims_no_update BEFORE UPDATE ON nullifier_claims BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER nullifier_claims_no_delete BEFORE DELETE ON nullifier_claims BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER outer_nonce_claims_no_update BEFORE UPDATE ON outer_nonce_claims BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER outer_nonce_claims_no_delete BEFORE DELETE ON outer_nonce_claims BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER events_no_update BEFORE UPDATE ON events BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER events_no_delete BEFORE DELETE ON events BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER incidents_no_update BEFORE UPDATE ON incidents BEGIN SELECT RAISE(ABORT,'immutable'); END;
CREATE TRIGGER incidents_no_delete BEFORE DELETE ON incidents BEGIN SELECT RAISE(ABORT,'immutable'); END;
