-- The FK parent is opened the same way, so `REFERENCES <an index>` is `42809` — the
-- shape a tables-map-only lookup answered `42P01`.
-- @setup
CREATE TABLE ril_t (a INT PRIMARY KEY);
CREATE INDEX ril_ix ON ril_t (a);
-- @test
CREATE TABLE ril_child (b INT REFERENCES ril_ix(a));
-- @expect-error: an index is not a referenceable relation (42809)
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
