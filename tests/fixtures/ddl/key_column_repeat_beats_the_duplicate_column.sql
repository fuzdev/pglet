-- Two gates answer `42701` on this statement, so the SQLSTATE separates nothing and
-- `@expect-message` is the whole assertion: the key list is resolved in parse
-- analysis and the column list in `DefineRelation`, so the statement must report
-- *appears twice in primary key constraint* and not *specified more than once*, in
-- either written order.
-- @expect-error: the repeated key column outranks the duplicate column
-- @expect-message
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE kcbdc (a INT, a INT, PRIMARY KEY (a, a));
