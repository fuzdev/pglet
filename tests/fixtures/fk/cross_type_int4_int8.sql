-- A child INT4 column REFERENCES a parent INT8 PK. Postgres RI coerces the child
-- value to the parent key type, so a valid reference is accepted. The covering-index
-- FK fast path must probe with the COERCED value (int8), not the child's own int4 key
-- encoding, which never matches the parent's int8 key (previously a wrong 23503).
-- @setup
CREATE TABLE cti_parent (id INT8 PRIMARY KEY);
CREATE TABLE cti_child (fk INT4 REFERENCES cti_parent(id));
INSERT INTO cti_parent VALUES (5), (42);

-- @test
INSERT INTO cti_child VALUES (5), (42);
SELECT fk FROM cti_child ORDER BY fk;
