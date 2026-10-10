-- PRIMARY KEY and FOREIGN KEY share the name pool too. The FK is added last, so its
-- name check is what reports — Postgres's `ATExecAddConstraint`, 42710.
-- @setup
CREATE TABLE ctpf_p (id INT PRIMARY KEY);

-- @expect-error: constraint m for relation ctpf already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ctpf (a INT NOT NULL, CONSTRAINT m PRIMARY KEY (a),
  CONSTRAINT m FOREIGN KEY (a) REFERENCES ctpf_p (id));
