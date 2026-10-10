-- A FROM alias and the qualifier that references it are both `ColId` names, so both
-- fold: `AS T` puts `t` in scope. Without the fold the qualified reference is
-- `missing FROM-clause entry for table "t"`.
-- @setup
CREATE TABLE talias (a INT);
INSERT INTO talias VALUES (1);

-- @test
SELECT t.a FROM talias AS T;
