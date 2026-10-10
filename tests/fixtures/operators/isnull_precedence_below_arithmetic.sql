-- `ISNULL` takes `IS`'s precedence (`%nonassoc IS ISNULL NOTNULL`), below `+`:
-- `1 + NULL ISNULL` is `(1 + NULL) ISNULL`, `t` — not `1 + (NULL ISNULL)`.
-- @test
SELECT 1 + NULL ISNULL AS v;
