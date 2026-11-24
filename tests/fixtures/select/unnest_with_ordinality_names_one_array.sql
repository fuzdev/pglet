-- `WITH ORDINALITY` adds a last `bigint` column numbering the rows from 1, named
-- `ordinality` when no alias list names it. Over one array the relation alias also names
-- the element column (`chooseScalarFunctionAlias`).
-- @test
SELECT * FROM unnest(ARRAY[10, 20, 30]) WITH ORDINALITY AS h ORDER BY h.ordinality DESC;
