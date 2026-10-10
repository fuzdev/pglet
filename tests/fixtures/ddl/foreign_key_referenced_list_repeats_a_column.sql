-- A foreign key's **referenced** list is the third independent key-list rule and has
-- its own SQLSTATE: 18.6's `transformFkeyCheckAttrs` refuses the repeat before it looks
-- for a covering index, because the list-to-index match would be ambiguous about which
-- opclass pairs with which key.
--
-- The parent carries a unique key on `x`, so the statement is well-formed in every other
-- respect — before this gate pglet accepted it and the foreign key then enforced.
-- @setup
CREATE TABLE fkrd_p (x INT, y INT, UNIQUE (x, y), UNIQUE (x));
-- @expect-error: the referenced list names x twice
-- @expect-message
-- @no-profile-lock: the @setup's indexes are the parent's enforced UNIQUE keys, built so the referenced list has something to resolve against; the refusal is a DDL-time catalog gate raised before any plan
-- @test
CREATE TABLE fkrd_c (a INT, b INT, FOREIGN KEY (a, b) REFERENCES fkrd_p (x, x));
