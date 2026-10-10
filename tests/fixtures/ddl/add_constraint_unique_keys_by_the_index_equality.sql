-- The `ALTER TABLE … ADD CONSTRAINT UNIQUE` half of the same rule, which had the same
-- structural basis and the same consequence: the constraint was recorded over rows it
-- does not hold. `ADD PRIMARY KEY` is the third route through `validate_existing_unique`
-- and answers identically (it adds only the `23502` NULL check, which this shape has no
-- NULL to reach).
-- @setup
CREATE TABLE acke_t (n NUMERIC);
INSERT INTO acke_t VALUES (2.000), (2.0), (2.00);
-- @test
ALTER TABLE acke_t ADD CONSTRAINT acke_u UNIQUE (n);
-- @expect-error: the index's own equality calls the three spellings one key (23505)
-- @expect-message
-- @no-profile-lock: DDL `@test` over a table with no index — index selection never runs
