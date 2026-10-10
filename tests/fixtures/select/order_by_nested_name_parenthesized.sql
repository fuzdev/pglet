-- The accept half of `order_by_nested_name_reads_the_base_column.sql`. Parentheses
-- build no node in Postgres's grammar, so `ORDER BY (q)` is still the one-field
-- `ColumnRef` `findTargetlistEntrySQL92` reaches and still resolves to the output
-- alias — `100 200 300`, the projected `a`, where the nested spelling `q+0` over the
-- identical statement answers `200 300 100`.
--
-- Keep both: an implementation that tests the key's source text rather than its parsed
-- shape passes one and fails the other.
-- @setup
CREATE TABLE onp (q INT, a INT);
INSERT INTO onp VALUES (3, 100), (1, 200), (2, 300);

-- @test
SELECT a AS q FROM onp ORDER BY (q);
