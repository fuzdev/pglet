-- The no-FROM leaf reaches the locking clause through its own call site, so
-- `FOR READ ONLY` is pinned there too.
-- @test
SELECT 1 AS q FOR READ ONLY;
