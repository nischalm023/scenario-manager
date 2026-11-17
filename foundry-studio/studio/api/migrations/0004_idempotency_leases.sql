-- The lease on an in-flight Idempotency-Key (spec 01 section 3.13's 409 while the first request runs; the lease is
-- lead decided, since the spec states no recovery from a crash). The request holding a key renews lease_expires_at
-- (wall clock) while its route runs. A key whose lease ran out, its process dead or its answer unstored, is taken
-- over by a repeat with the same digest, which runs again, instead of answering 409 until the 24-hour purge. Each
-- taking of a key mints owner_token, and a request stores or releases the key only under its own token, so a request
-- whose claim lapsed never settles the claim that replaced it.

ALTER TABLE idempotency_keys
    ADD COLUMN owner_token      UUID,
    ADD COLUMN lease_expires_at TIMESTAMPTZ;

-- A key in flight before this migration was taken by a build that renewed no lease: its lease has run out.
UPDATE idempotency_keys SET owner_token = gen_random_uuid(), lease_expires_at = now() WHERE state = 'in_flight';

ALTER TABLE idempotency_keys
    ADD CONSTRAINT idempotency_keys_in_flight_holds_a_lease
        CHECK (state <> 'in_flight' OR (owner_token IS NOT NULL AND lease_expires_at IS NOT NULL));
