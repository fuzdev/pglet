-- An owned sequence whose column carries no default has only the AUTO edge, so it
-- drops cleanly and its owner table is untouched — the accept side of the two
-- refusals. The re-`CREATE` under the same name would be 42P07 if the drop had not
-- happened.
-- @setup
CREATE TABLE dsow_t (a INT);
CREATE SEQUENCE dsow_seq START WITH 50;
ALTER SEQUENCE dsow_seq OWNED BY dsow_t.a;
DROP SEQUENCE dsow_seq;
CREATE SEQUENCE dsow_seq START WITH 8;
INSERT INTO dsow_t (a) VALUES (nextval('dsow_seq'));
-- @test
SELECT a FROM dsow_t;
