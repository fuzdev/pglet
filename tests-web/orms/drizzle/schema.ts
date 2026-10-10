/**
 * Drizzle schema for pglet compatibility testing.
 *
 * Simple generic schema to test ORM operations.
 */

import {
  boolean,
  integer,
  pgTable,
  text,
  timestamp,
} from "drizzle-orm/pg-core";

export const users = pgTable("users", {
  id: integer("id").primaryKey(),
  name: text("name").notNull(),
  email: text("email").notNull(),
  active: boolean("active").default(true),
  created_at: timestamp("created_at"),
});

export const posts = pgTable("posts", {
  id: integer("id").primaryKey(),
  title: text("title").notNull(),
  content: text("content"),
  author_id: integer("author_id").notNull(),
  published: boolean("published").default(false),
});
