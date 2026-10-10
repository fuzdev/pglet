-- A restart moves the counter and nothing else, and a **second** `RESTART IDENTITY` is
-- what shows it. Replacing the record with a fresh one at `start` — the shape a
-- counter-only sequence model invites, and the one `setval` and this path both once
-- had — silently drops the ownership edge, after which the second truncate finds
-- nothing to restart and the counter climbs. One truncate cannot tell the two apart:
-- both answer 50.
-- @setup
CREATE TABLE trtw_t (a INT);
CREATE SEQUENCE trtw_seq START WITH 50;
ALTER SEQUENCE trtw_seq OWNED BY trtw_t.a;
INSERT INTO trtw_t (a) VALUES (nextval('trtw_seq'));
TRUNCATE trtw_t RESTART IDENTITY;
INSERT INTO trtw_t (a) VALUES (nextval('trtw_seq'));
TRUNCATE trtw_t RESTART IDENTITY;
-- @test
SELECT nextval('trtw_seq');
