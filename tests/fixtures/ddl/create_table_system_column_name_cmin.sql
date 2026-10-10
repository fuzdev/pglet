-- One member of 18.6's `SysAtt[]` — the six names a relation's heap already owns, so
-- no user column may take one. Each of the six has its own fixture, split across the
-- two statements that add a name to a relation's column list, because the **list** is
-- what this class gets wrong: a short enumeration reddens nothing, and only the oracle
-- can say a name is missing from it.
-- @expect-error: cmin is a system column name
-- @expect-message
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE ctsc_cmin (cmin INT);
