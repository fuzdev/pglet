-- `GENERATED … AS IDENTITY` is not a default and takes its own sentence — `42601
-- both default and identity specified` — in either written order.
-- @expect-error: both default and identity specified for column "a"
-- @test
CREATE TABLE def_ident (a INT DEFAULT 5 GENERATED ALWAYS AS IDENTITY);
