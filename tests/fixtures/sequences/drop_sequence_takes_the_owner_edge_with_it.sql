-- Dropping an owned sequence removes the ownership edge with it, so a later
-- `TRUNCATE … RESTART IDENTITY` on the ex-owner has nothing to restart — the counter
-- of the *replacement* sequence, which nothing owns, keeps climbing.
-- @setup
CREATE TABLE dsto_t (a INT);
CREATE SEQUENCE dsto_seq START WITH 1;
ALTER SEQUENCE dsto_seq OWNED BY dsto_t.a;
DROP SEQUENCE dsto_seq;
CREATE SEQUENCE dsto_seq START WITH 1;
INSERT INTO dsto_t (a) VALUES (nextval('dsto_seq'));
TRUNCATE dsto_t RESTART IDENTITY;
-- @test
SELECT nextval('dsto_seq');
