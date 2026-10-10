-- The parent TIMESTAMP is NOT at midnight, so the child DATE (which casts to midnight)
-- has no matching parent — 23503, matching Postgres. The cross-type coercion is exact
-- (date -> midnight timestamp), never a fuzzy same-day match.
-- @profile-lock
-- @setup
CREATE TABLE cdta_parent (ts TIMESTAMP PRIMARY KEY);
CREATE TABLE cdta_child (d DATE REFERENCES cdta_parent(ts));
INSERT INTO cdta_parent VALUES ('2024-01-01 12:00:00');

-- @test
-- @expect-error: the date casts to midnight, not the parent's noon timestamp
INSERT INTO cdta_child VALUES ('2024-01-01');
