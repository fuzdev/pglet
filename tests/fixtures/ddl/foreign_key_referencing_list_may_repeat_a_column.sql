-- The control that bounds the referenced-list rule to the **referenced** side: the
-- *referencing* list may name one child column twice on both engines, and the foreign
-- key then enforces the pair `(a, a)` against the parent's `(x, y)` key. A gate that
-- deduplicated both sides of a foreign key reddens here; its `CREATE INDEX` sibling
-- (`create_index_key_may_repeat_a_column.sql`) is the other half of that bound.
-- @setup
CREATE TABLE fkrg_p (x INT, y INT, PRIMARY KEY (x, y));
INSERT INTO fkrg_p VALUES (1, 1), (2, 3);
CREATE TABLE fkrg_c (a INT, FOREIGN KEY (a, a) REFERENCES fkrg_p (x, y));
INSERT INTO fkrg_c VALUES (1);
-- @test
SELECT a FROM fkrg_c ORDER BY a;
