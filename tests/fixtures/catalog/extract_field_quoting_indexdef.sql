-- `pg_indexes.indexdef` reads the same renderer for an index's key expression and for
-- its predicate, so an `EXTRACT` field is run through `quote_identifier` at both: an
-- uppercase, mixed-case or keyword string constant comes back quoted, a bare field word
-- and a lowercase string constant bare. `date` and `time` columns, because over a
-- `timestamptz` the predicate is not `IMMUTABLE` and 18.6 refuses the index.
-- `extract_field_quoting_constraintdef.sql` names the rule's members.
-- @setup
CREATE TABLE xi (d DATE, ti TIME);
CREATE INDEX xi_a_idx ON xi ((EXTRACT('MONTH' FROM d))) WHERE EXTRACT('Hour' FROM ti) > 1.0;
CREATE INDEX xi_b_idx ON xi ((EXTRACT(MONTH FROM d))) WHERE EXTRACT('hour' FROM ti) > 1.0;
CREATE INDEX xi_c_idx ON xi ((EXTRACT('select' FROM d))) WHERE EXTRACT('my"x' FROM ti) IS NOT NULL;
-- @test
SELECT indexname, indexdef FROM pg_indexes
WHERE tablename = 'xi' AND indexname LIKE '%\_idx' ORDER BY indexname
