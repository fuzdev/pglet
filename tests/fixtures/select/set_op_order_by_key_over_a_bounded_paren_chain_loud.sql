-- A parenthesized operand's own `LIMIT` does not change what the trailing `ORDER BY` may
-- name: 18.6 is `42703` here exactly as it is without the bound, so the set-op test the
-- gate keys on peels a `Plan::Limit`. An operand carrying its own `ORDER BY` as well is
-- `42601 multiple ORDER BY clauses not allowed` on both engines, refused in the parse
-- before the gate runs (`EDGES.md` row 30).
-- @setup
CREATE TABLE sopa (a INT);
CREATE TABLE sopb (a INT);

-- @expect-error: column "bogus" does not exist (42703)
-- @test
(SELECT a FROM sopa UNION SELECT a FROM sopb LIMIT 1) ORDER BY bogus;
