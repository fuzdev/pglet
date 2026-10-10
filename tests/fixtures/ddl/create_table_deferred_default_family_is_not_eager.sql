-- The cross-route control for the four `add_column_default_*_is_eager` fixtures: the
-- **identical** four defaults on a `CREATE TABLE` are accepted on both engines, and
-- the column carries the default until a row is written. Without this row the eager
-- refusals are satisfied by an engine that refuses the family everywhere — which is
-- what a value-dependent check placed in the shared cook rather than at the one route
-- would produce.
-- @setup
CREATE TABLE ct_deferred (
    a INT,
    b INT DEFAULT 1/0,
    c INT DEFAULT (2147483647 + 1),
    d VARCHAR(3) DEFAULT 'abcdef',
    e NUMERIC(4,2) DEFAULT 12345.6
);
INSERT INTO ct_deferred (a, b, c, d, e) VALUES (1, 2, 3, 'ab', 1.5);
-- @test
SELECT a, b, c, d, e FROM ct_deferred;
