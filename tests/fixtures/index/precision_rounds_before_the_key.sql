-- **The store rounds before the index key is encoded**, so two literals that differ only
-- below the column's precision collide under a UNIQUE index — the same row 18.6 refuses,
-- under the same `DETAIL` naming the *rounded* key.
--
-- This is the one shape that pins the order of the two: an encoder reading the value
-- before `coerce_column_value` had rounded it would index two distinct keys and accept
-- the second row.
-- @setup
CREATE TABLE ixp (b interval(3));
CREATE UNIQUE INDEX ixp_b_key ON ixp (b);
INSERT INTO ixp VALUES ('1.23456 secs');
-- `@profile-lock` rather than the justified opt-out: the claim it would have to make —
-- that a unique index is *enforced* on both build profiles, only query-time acceleration
-- being behind the feature — is one the lock run measures instead.
-- @profile-lock
-- @expect-error: a second row whose value rounds onto the first
-- @test
INSERT INTO ixp VALUES ('1.2345 secs');
