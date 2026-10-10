-- The other half of the ambiguity rule, one namespace over: two **target-list entries**
-- under one output name. Postgres's `findTargetlistEntrySQL92` scans the whole target list
-- and raises `42702` unless the two matching entries are `equal()`; here they are `c` and
-- `x`, two different columns, so it refuses. A check that does not read the target list
-- answers `(0 rows)` here over the empty relation.
--
-- Its accept twins are `select/order_by_alias_and_bare_duplicate_resolve.sql` and
-- `select/order_by_star_and_bare_duplicate_resolve.sql`, where the two entries denote one
-- column and both engines answer — this file beside either is what shows the rule is
-- `equal()` over the entries and not a count of the name.
-- @setup
CREATE TABLE sk_amb (c INT, x TEXT);

-- @expect-error: ORDER BY "q" is ambiguous (42702)
-- @test
SELECT c AS q, x AS q FROM sk_amb ORDER BY q;
