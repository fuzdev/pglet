-- Composite FOREIGN KEY under MATCH SIMPLE (the default): if ANY referencing column
-- is NULL, the constraint is skipped entirely — the row is allowed even though the
-- partial tuple has no matching parent.
-- @profile-lock
-- @setup
CREATE TABLE cms_parent (x INT, y INT, PRIMARY KEY (x, y));
CREATE TABLE cms_child (
  id INT PRIMARY KEY,
  a INT,
  b INT,
  FOREIGN KEY (a, b) REFERENCES cms_parent (x, y)
);
INSERT INTO cms_parent VALUES (1, 2);
INSERT INTO cms_child VALUES (1, 1, 2);       -- full match — allowed
INSERT INTO cms_child VALUES (2, 1, NULL);    -- partial NULL — MATCH SIMPLE skips → allowed
INSERT INTO cms_child VALUES (3, NULL, NULL); -- both NULL — allowed

-- @test
SELECT id, a, b FROM cms_child ORDER BY id;
