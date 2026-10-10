-- The `Key (…)=(…)` values are rendered under the referencing column's **declared**
-- type, so a `timestamptz` keeps its `+00`. The `ALTER TABLE … ADD FOREIGN KEY`
-- validation scan built its key description through the type-blind `value_to_text` and
-- dropped the offset — one field, both engines sending it, different values. The INT-keyed
-- sibling fixture could not see it.
-- @profile-lock
-- @setup
CREATE TABLE tz_p (ts TIMESTAMPTZ PRIMARY KEY);
CREATE TABLE tz_c (ts TIMESTAMPTZ);
INSERT INTO tz_c VALUES ('2024-01-02 03:04:05+00');

-- @test
-- @expect-error: an existing row references no parent
-- @expect-message
ALTER TABLE tz_c ADD CONSTRAINT tz_c_fk FOREIGN KEY (ts) REFERENCES tz_p(ts);
