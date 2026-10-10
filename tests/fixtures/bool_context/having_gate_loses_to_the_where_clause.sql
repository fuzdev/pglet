-- Its mirror, likewise cross-code: the `WHERE`'s own binding failure is reported
-- where a `HAVING` gate running first would answer `42804`.
-- @setup
CREATE TABLE bcord8 (k INT);
-- @expect-error: the WHERE's 42703, not the HAVING gate's 42804
-- @test
SELECT max(k) FROM bcord8 WHERE nosuchcol HAVING 1;
