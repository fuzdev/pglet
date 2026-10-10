-- `gram.y:14345` `for_locking_clause: for_locking_items | FOR READ ONLY` — the two are
-- **alternatives**, so `FOR READ ONLY` cannot follow a locking item. This is the order
-- an arm placed one loop iteration too late admitted, answering rows.
-- @expect-error: FOR READ ONLY cannot follow a locking item
-- @setup
CREATE TABLE lro2 (id INT, code TEXT);
INSERT INTO lro2 VALUES (1, 'a');
-- @test
SELECT id FROM lro2 FOR UPDATE FOR READ ONLY;
