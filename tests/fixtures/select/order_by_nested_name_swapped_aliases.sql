-- Both names are output names *and* base columns, swapped — so a key resolved against
-- the output and one resolved against the base row disagree in both directions at once
-- and neither engine errors. `ORDER BY a+0` is the base `a` (1, 2), giving `20|1 10|2`;
-- resolving the nested `a` against the output takes the projected `b` (20, 10) and
-- reverses the pair.
--
-- The projection carries *both* columns here, which is what separates this from
-- `order_by_nested_name_reads_the_base_column.sql`: no column is dropped, so the sort
-- has everything it needs in either layout and only the *binding* decides the answer.
-- @setup
CREATE TABLE ons (a INT, b INT);
INSERT INTO ons VALUES (1, 20), (2, 10);

-- @test
SELECT b AS a, a AS b FROM ons ORDER BY a+0;
