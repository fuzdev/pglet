-- ...but *not* `pg_constraint`: `transformColumnDefinition` calls
-- `ChooseRelationName(..., isconstraint = false)`, so a CHECK constraint named
-- `<t>_<c>_seq` leaves the sequence on its plain name. The id is 1 either way, so
-- the sequence's own name is what the test reads.
-- @setup
CREATE TABLE conname_host (a INT CONSTRAINT conndis_b_seq CHECK (a > 0));
CREATE TABLE conndis (b SERIAL);
INSERT INTO conndis (b) VALUES (DEFAULT);
-- @test
SELECT nextval('conndis_b_seq') AS next_from_the_columns_sequence;
