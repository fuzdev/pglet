-- @no-txn
-- Drives its own BEGIN/ROLLBACK, so it opts out of the harness's isolating wrapper.
--
-- The third cell of the TRUNCATE rollback, and the one neither sibling could see.
-- `rollback_truncate_restart_identity.sql` pins that a restart of an **owned** sequence
-- is undone; `rollback_plain_truncate_keeps_a_sequence_advance.sql` pins that a plain
-- TRUNCATE replaces nothing. Between them sits `RESTART IDENTITY` over a table owning
-- **no** sequence, which replaces no cell either — so an unrelated `nextval` the same
-- transaction issued stands after the ROLLBACK: 2 on both engines, two keywords away
-- from the plain form.
--
-- `Plan::swapped_sequences` reads `Schema::truncation_targets` — the owned-sequence set
-- `execute_truncate` writes from — rather than the statement's keywords. A reading keyed
-- on the keyword that lists sequences the truncated table does not own would record
-- `rriwo_b`'s cell here and turn this fixture red while its plain sibling stays green
-- (measured through the embed facade: `1|2`), and a rollback that restored every counter
-- from its frame would turn both red.
-- @setup
CREATE TABLE rriwo_a (v INT);
CREATE TABLE rriwo_b (id INT GENERATED ALWAYS AS IDENTITY, v INT);
BEGIN;
TRUNCATE rriwo_a RESTART IDENTITY;
INSERT INTO rriwo_b (v) VALUES (1);
ROLLBACK;
INSERT INTO rriwo_b (v) VALUES (2);
-- @test
SELECT id, v FROM rriwo_b ORDER BY id;
