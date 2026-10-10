-- `ISNULL` binds below `=` and `LIKE`: `NULL = NULL ISNULL` is `(NULL = NULL) ISNULL`
-- (`t`, not NULL), and `'a' LIKE 'a' ISNULL` is `f`.
-- @test
SELECT NULL = NULL ISNULL AS eq, 'a' LIKE 'a' ISNULL AS lk, 1 ISNULL = false AS cmp;
