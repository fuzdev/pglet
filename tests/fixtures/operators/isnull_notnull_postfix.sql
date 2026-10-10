-- `x ISNULL` / `x NOTNULL` are `gram.y`'s one-word spellings of `IS [NOT] NULL` — the
-- null test — not an operand followed by an output label.
-- @test
SELECT NULL ISNULL AS a, 1 ISNULL AS b, NULL NOTNULL AS c, 1 NOTNULL AS d;
