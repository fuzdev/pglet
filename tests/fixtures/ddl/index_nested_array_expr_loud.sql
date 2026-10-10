-- An expression index whose key builds a nested array used to be **accepted** and then
-- brick the table: every subsequent INSERT failed `0A000` forever (the index
-- maintenance path cannot encode an array-of-array key), surviving restart and
-- recoverable only by `DROP INDEX`. It was the fourth accept-then-brick of its kind.
-- Rejecting the nested array itself closes this route at DDL time, where Postgres puts
-- its own failures — though Postgres accepts *this* statement, since it supports
-- multidimensional arrays, so pglet is deliberately stricter.
-- @setup
CREATE TABLE nxi (id INT);
-- @test
-- @expect-error: nested-array index key rejected at DDL (Postgres accepts it)
CREATE INDEX nxi_nested ON nxi ((ARRAY[ARRAY[1,2]]));
