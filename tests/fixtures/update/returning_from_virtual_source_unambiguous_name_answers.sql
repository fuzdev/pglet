-- The **accept** polarity of `returning_from_virtual_source_column_ambiguous.sql`.
--
-- `x` is a column the catalog-view source does not carry, so it has one claimant and
-- binds — on 18.6 and here. Registering the source as a claimant must not make every
-- bare `RETURNING` name over a virtual source an error, and a fix that sealed the scope
-- whenever the `FROM` held a kind it could not previously enumerate reddens here and
-- nowhere else.
-- @setup
CREATE TABLE rvu (table_name TEXT, x INT);
INSERT INTO rvu VALUES ('rvu', 0);

-- @test
UPDATE rvu SET x = 1 FROM information_schema.tables c
  WHERE c.table_name = rvu.table_name RETURNING x;
