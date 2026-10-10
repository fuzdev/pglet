-- @setup
CREATE TABLE wz (a INT, b INT);
INSERT INTO wz VALUES (1, 5), (1, 7), (2, 5);

-- The whole `order_by_nested_name_*` family, one node up: a name nested inside an
-- `ORDER BY` key resolves against the FROM relations, never the output alias
-- (`findTargetlistEntrySQL92`'s one-field gate). A `Plan::Window` cannot take the
-- sort-below layout the `Plan::Project` members use — a window function is computed
-- over the *input* order — so the key rides the Window as a junk column instead.
-- `b AS a` shadows the base `a`, and the two run in different orders, so binding the
-- output silently answers `5 5 7` where the base column answers `5 7 5`.
-- @test
SELECT b AS a, row_number() OVER () AS rn FROM wz ORDER BY a + 0;
