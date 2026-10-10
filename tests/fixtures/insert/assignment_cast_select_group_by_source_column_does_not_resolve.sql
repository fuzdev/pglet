-- **A false-rejection control — do not trim it.** A `GROUP BY` key that names a
-- **source** column is its own group expression: it adds a `SortGroupClause` over that
-- column and never touches the `'t'` target-list entry, which stays `unknown` for the
-- target column to resolve. Accepted on 18.6.
--
-- Its refuse twin is
-- `assignment_cast_select_group_by_ordinal_resolves_the_unknown_loud.sql` — the same
-- statement with `GROUP BY 1`. Marking an aggregate's whole target list, rather than
-- the entries its keys name, refuses this one.
-- @setup
CREATE TABLE acgs (id INT, b BOOL);
CREATE TABLE acgssrc (n INT);
INSERT INTO acgssrc VALUES (1), (1), (2);
-- @test
INSERT INTO acgs (b) SELECT 't' FROM acgssrc GROUP BY n RETURNING b;
