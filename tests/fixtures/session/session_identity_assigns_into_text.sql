-- Every type assigns into a string-category target on 18.6 (I/O coercion), so a
-- `name`-typed expression stores into a `text` column. The `varchar(20)` column is
-- the second target and also checks the typmod path does not reject it.
-- @setup
CREATE TABLE sa_store (t TEXT, v VARCHAR(20));
INSERT INTO sa_store (t, v) VALUES (current_user, current_schema);
-- @test
SELECT t, v FROM sa_store;
