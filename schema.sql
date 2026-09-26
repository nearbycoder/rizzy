PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS app_secrets (
  name TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS accounts (
  id TEXT PRIMARY KEY,
  external_account_id INTEGER NOT NULL UNIQUE,
  name TEXT NOT NULL,
  cards_count INTEGER NOT NULL DEFAULT 0,
  auto_postpone_period_in_days INTEGER NOT NULL DEFAULT 30,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS account_cancellations (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL UNIQUE REFERENCES accounts(id) ON DELETE CASCADE,
  initiated_by_id TEXT NOT NULL REFERENCES users(id),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS account_cancellations_due ON account_cancellations(created_at);
CREATE TABLE IF NOT EXISTS account_join_codes (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL UNIQUE REFERENCES accounts(id) ON DELETE CASCADE,
  code TEXT NOT NULL UNIQUE,
  usage_count INTEGER NOT NULL DEFAULT 0,
  usage_limit INTEGER NOT NULL DEFAULT 10,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS identities (
  id TEXT PRIMARY KEY,
  email_address TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS identity_transfers (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
  token TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS identity_transfers_identity_created ON identity_transfers(identity_id,created_at DESC);
CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  identity_id TEXT REFERENCES identities(id),
  name TEXT NOT NULL,
  role TEXT NOT NULL DEFAULT 'member',
  active INTEGER NOT NULL DEFAULT 1,
  verified_at TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT '',
  UNIQUE(account_id, identity_id)
);
CREATE UNIQUE INDEX IF NOT EXISTS users_system_account ON users(account_id) WHERE role='system';
CREATE TABLE IF NOT EXISTS user_attachables (
  user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  sgid TEXT NOT NULL UNIQUE
);
CREATE TRIGGER IF NOT EXISTS user_attachables_created AFTER INSERT ON users BEGIN
  INSERT INTO user_attachables(user_id,account_id,sgid) VALUES(NEW.id,NEW.account_id,lower(hex(randomblob(32))));
END;
CREATE TABLE IF NOT EXISTS access_tokens (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL REFERENCES identities(id),
  token_hash TEXT NOT NULL UNIQUE,
  description TEXT NOT NULL,
  permission TEXT NOT NULL CHECK(permission IN ('read','write')),
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS access_token_reveals (
  id TEXT PRIMARY KEY,
  access_token_id TEXT NOT NULL REFERENCES access_tokens(id) ON DELETE CASCADE,
  identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
  token TEXT NOT NULL,
  expires_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS access_token_reveals_expiration ON access_token_reveals(expires_at);
CREATE TABLE IF NOT EXISTS magic_links (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
  code TEXT NOT NULL UNIQUE,
  purpose TEXT NOT NULL CHECK(purpose IN ('sign_in','sign_up')),
  expires_at TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS magic_links_expiration ON magic_links(expires_at);
CREATE TABLE IF NOT EXISTS email_change_requests (
  token_hash TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  old_identity_id TEXT NOT NULL REFERENCES identities(id),
  new_email_address TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS email_change_requests_expiration ON email_change_requests(expires_at);
CREATE TABLE IF NOT EXISTS pending_authentications (
  token_hash TEXT PRIMARY KEY,
  email_address TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS pending_authentications_expiration ON pending_authentications(expires_at);
CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
  token_hash TEXT NOT NULL UNIQUE,
  user_agent TEXT,
  signup_pending INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS passkeys (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
  credential_id TEXT NOT NULL UNIQUE,
  credential_json TEXT NOT NULL,
  name TEXT,
  transports TEXT NOT NULL DEFAULT '[]',
  sign_count INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS passkeys_identity_name ON passkeys(identity_id,name,created_at DESC);
CREATE TABLE IF NOT EXISTS passkey_challenges (
  challenge TEXT PRIMARY KEY,
  purpose TEXT NOT NULL CHECK(purpose IN ('registration','authentication')),
  identity_id TEXT REFERENCES identities(id) ON DELETE CASCADE,
  state_json TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS passkey_challenges_expiration ON passkey_challenges(expires_at);
CREATE INDEX IF NOT EXISTS sessions_identity ON sessions(identity_id);
CREATE TABLE IF NOT EXISTS authentication_attempts (
  id TEXT PRIMARY KEY,
  bucket TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS authentication_attempts_bucket ON authentication_attempts(bucket,created_at);
CREATE INDEX IF NOT EXISTS authentication_attempts_age ON authentication_attempts(created_at);
CREATE TABLE IF NOT EXISTS exports (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK(kind IN ('user','account')),
  status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','processing','completed','failed')),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  completed_at TEXT
);
CREATE INDEX IF NOT EXISTS exports_current ON exports(account_id,kind,created_at);
CREATE TABLE IF NOT EXISTS imports (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  identity_id TEXT NOT NULL REFERENCES identities(id),
  status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','processing','completed','failed')),
  failure_reason TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  completed_at TEXT
);
CREATE INDEX IF NOT EXISTS imports_pending ON imports(status,created_at);
CREATE TABLE IF NOT EXISTS boards (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  creator_id TEXT NOT NULL REFERENCES users(id),
  name TEXT NOT NULL,
  all_access INTEGER NOT NULL DEFAULT 1,
  auto_postpone_period_in_days INTEGER NOT NULL DEFAULT 30,
  entropy_override INTEGER NOT NULL DEFAULT 0,
  public_description TEXT NOT NULL DEFAULT '',
  publication_key TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS boards_account ON boards(account_id, created_at DESC);
CREATE UNIQUE INDEX IF NOT EXISTS boards_publication_key ON boards(publication_key) WHERE publication_key IS NOT NULL;
CREATE TABLE IF NOT EXISTS board_publications (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  board_id TEXT NOT NULL UNIQUE REFERENCES boards(id) ON DELETE CASCADE,
  key TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS board_publications_account ON board_publications(account_id);
CREATE TABLE IF NOT EXISTS entropy_records (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  container_type TEXT NOT NULL CHECK(container_type IN ('Account','Board')),
  container_id TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(container_type,container_id)
);
CREATE INDEX IF NOT EXISTS entropy_records_account ON entropy_records(account_id);
CREATE TRIGGER IF NOT EXISTS entropy_records_board_delete AFTER DELETE ON boards BEGIN
  DELETE FROM entropy_records WHERE container_type='Board' AND container_id=OLD.id;
END;
CREATE TABLE IF NOT EXISTS accesses (
  board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  involvement TEXT NOT NULL DEFAULT 'access_only' CHECK(involvement IN ('access_only','watching')),
  accessed_at TEXT,
  PRIMARY KEY(board_id, user_id)
);
CREATE INDEX IF NOT EXISTS accesses_user_board ON accesses(user_id,board_id);
CREATE INDEX IF NOT EXISTS accesses_user_recent ON accesses(user_id,accessed_at DESC);
CREATE TABLE IF NOT EXISTS access_records (
  board_id TEXT NOT NULL,
  user_id TEXT NOT NULL,
  id TEXT NOT NULL UNIQUE,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY(board_id,user_id),
  FOREIGN KEY(board_id,user_id) REFERENCES accesses(board_id,user_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS access_records_account ON access_records(account_id);
CREATE INDEX IF NOT EXISTS access_records_user_account ON access_records(user_id,account_id);
CREATE TABLE IF NOT EXISTS columns (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  color TEXT NOT NULL DEFAULT 'var(--color-card-default)',
  position INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS columns_board_position ON columns(board_id, position);
CREATE TABLE IF NOT EXISTS cards (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  column_id TEXT REFERENCES columns(id) ON DELETE SET NULL,
  creator_id TEXT NOT NULL REFERENCES users(id),
  number INTEGER NOT NULL,
  title TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  search_text TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'published',
  due_on TEXT,
  closed INTEGER NOT NULL DEFAULT 0,
  postponed INTEGER NOT NULL DEFAULT 0,
  golden INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  last_active_at TEXT NOT NULL,
  UNIQUE(account_id, number)
);
CREATE INDEX IF NOT EXISTS cards_board_active ON cards(board_id, status, closed, postponed, last_active_at DESC);
CREATE INDEX IF NOT EXISTS cards_board_column_active ON cards(board_id, column_id, status, closed, postponed, golden DESC, last_active_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS cards_account_latest ON cards(account_id, last_active_at DESC);
CREATE TABLE IF NOT EXISTS card_bulk_touches (
  board_id TEXT PRIMARY KEY REFERENCES boards(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS storage_blobs (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  storage_key TEXT NOT NULL UNIQUE,
  filename TEXT NOT NULL,
  content_type TEXT NOT NULL,
  byte_size INTEGER NOT NULL,
  checksum TEXT NOT NULL,
  metadata TEXT NOT NULL DEFAULT '{}',
  signed_id TEXT NOT NULL UNIQUE,
  attachable_sgid TEXT NOT NULL UNIQUE,
  upload_token_hash TEXT NOT NULL UNIQUE,
  upload_expires_at TEXT NOT NULL,
  uploaded_at TEXT,
  width INTEGER,
  height INTEGER,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS storage_variant_records (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  blob_id TEXT NOT NULL REFERENCES storage_blobs(id) ON DELETE CASCADE,
  variation_digest TEXT NOT NULL,
  UNIQUE(blob_id,variation_digest)
);
CREATE TABLE IF NOT EXISTS storage_attachments (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  blob_id TEXT NOT NULL REFERENCES storage_blobs(id) ON DELETE CASCADE,
  record_type TEXT NOT NULL,
  record_id TEXT NOT NULL,
  name TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS storage_card_image_unique
  ON storage_attachments(record_id) WHERE record_type='Card' AND name='image';
CREATE UNIQUE INDEX IF NOT EXISTS storage_card_image_blob_unique
  ON storage_attachments(blob_id) WHERE record_type='Card' AND name='image';
CREATE UNIQUE INDEX IF NOT EXISTS storage_user_avatar_unique
  ON storage_attachments(record_id) WHERE record_type='User' AND name='avatar';
CREATE UNIQUE INDEX IF NOT EXISTS storage_user_avatar_blob_unique
  ON storage_attachments(blob_id) WHERE record_type='User' AND name='avatar';
CREATE UNIQUE INDEX IF NOT EXISTS storage_embed_unique
  ON storage_attachments(record_type,record_id,name,blob_id) WHERE name='embed';
CREATE INDEX IF NOT EXISTS storage_attachments_record ON storage_attachments(record_type,record_id);
CREATE TABLE IF NOT EXISTS storage_download_tokens (
  token_hash TEXT PRIMARY KEY,
  blob_id TEXT NOT NULL REFERENCES storage_blobs(id) ON DELETE CASCADE,
  disposition TEXT NOT NULL DEFAULT 'inline',
  variant TEXT NOT NULL DEFAULT 'original',
  expires_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS closures (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  card_id TEXT NOT NULL UNIQUE REFERENCES cards(id) ON DELETE CASCADE,
  user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS closures_user_created ON closures(user_id,created_at);
CREATE TABLE IF NOT EXISTS card_goldnesses (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  card_id TEXT NOT NULL UNIQUE REFERENCES cards(id) ON DELETE CASCADE,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS card_goldnesses_account ON card_goldnesses(account_id);
CREATE TABLE IF NOT EXISTS card_not_nows (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  card_id TEXT NOT NULL UNIQUE REFERENCES cards(id) ON DELETE CASCADE,
  user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS card_not_nows_account ON card_not_nows(account_id);
CREATE INDEX IF NOT EXISTS card_not_nows_user ON card_not_nows(user_id);
CREATE TABLE IF NOT EXISTS card_activity_spikes (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  card_id TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS card_activity_spikes_account_updated ON card_activity_spikes(account_id,updated_at);
CREATE TRIGGER IF NOT EXISTS cards_delete_activity_spikes AFTER DELETE ON cards
BEGIN
  DELETE FROM card_activity_spikes WHERE card_id=OLD.id;
END;
CREATE TABLE IF NOT EXISTS steps (
  id TEXT PRIMARY KEY,
  card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
  content TEXT NOT NULL,
  completed INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS steps_card ON steps(card_id);
CREATE TABLE IF NOT EXISTS comments (
  id TEXT PRIMARY KEY,
  card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
  creator_id TEXT NOT NULL REFERENCES users(id),
  body TEXT NOT NULL,
  search_text TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  updated_at TEXT
);
CREATE INDEX IF NOT EXISTS comments_card_created ON comments(card_id,created_at);
CREATE TABLE IF NOT EXISTS rich_text_records (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  record_type TEXT NOT NULL,
  record_id TEXT NOT NULL,
  name TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  body_is_null INTEGER NOT NULL DEFAULT 0,
  UNIQUE(record_type,record_id,name)
);
CREATE INDEX IF NOT EXISTS rich_text_records_account ON rich_text_records(account_id);
CREATE TRIGGER IF NOT EXISTS cards_delete_rich_text AFTER DELETE ON cards
BEGIN
  DELETE FROM rich_text_records WHERE record_type='Card' AND record_id=OLD.id;
END;
CREATE TRIGGER IF NOT EXISTS comments_delete_rich_text AFTER DELETE ON comments
BEGIN
  DELETE FROM rich_text_records WHERE record_type='Comment' AND record_id=OLD.id;
END;
CREATE TRIGGER IF NOT EXISTS boards_delete_rich_text AFTER DELETE ON boards
BEGIN
  DELETE FROM rich_text_records WHERE record_type='Board' AND record_id=OLD.id;
END;
CREATE TABLE IF NOT EXISTS tags (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  title TEXT NOT NULL,
  created_at TEXT NOT NULL,
  UNIQUE(account_id,title)
);
CREATE TABLE IF NOT EXISTS taggings (
  card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
  tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  PRIMARY KEY(card_id,tag_id)
);
CREATE TABLE IF NOT EXISTS assignments (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
  assignee_id TEXT NOT NULL REFERENCES users(id),
  assigner_id TEXT NOT NULL REFERENCES users(id),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT '',
  UNIQUE(card_id,assignee_id)
);
CREATE INDEX IF NOT EXISTS assignments_assignee ON assignments(assignee_id,card_id);
CREATE TABLE IF NOT EXISTS watches (
  id TEXT NOT NULL UNIQUE,
  card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id),
  watching INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY(card_id,user_id)
);
CREATE TABLE IF NOT EXISTS pins (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id),
  created_at TEXT NOT NULL,
  UNIQUE(user_id,card_id)
);
CREATE INDEX IF NOT EXISTS pins_user_recent ON pins(user_id,created_at DESC);
CREATE TABLE IF NOT EXISTS reactions (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  reactable_type TEXT NOT NULL CHECK(reactable_type IN ('Card','Comment')),
  reactable_id TEXT NOT NULL,
  reacter_id TEXT NOT NULL REFERENCES users(id),
  content TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS reactions_target ON reactions(reactable_type,reactable_id,created_at);
CREATE TABLE IF NOT EXISTS mentions (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  mentionee_id TEXT NOT NULL REFERENCES users(id),
  mentioner_id TEXT NOT NULL REFERENCES users(id),
  source_type TEXT NOT NULL CHECK(source_type IN ('Card','Comment')),
  source_id TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS mentions_source ON mentions(source_type,source_id);
CREATE INDEX IF NOT EXISTS mentions_mentionee ON mentions(mentionee_id);
CREATE TABLE IF NOT EXISTS events (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
  creator_id TEXT NOT NULL REFERENCES users(id),
  eventable_type TEXT NOT NULL CHECK(eventable_type IN ('Card','Comment')),
  eventable_id TEXT NOT NULL,
  action TEXT NOT NULL,
  particulars TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS events_account_recent ON events(account_id,created_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS events_board_recent ON events(board_id,created_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS events_eventable ON events(eventable_type,eventable_id);
CREATE TABLE IF NOT EXISTS notifications (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  creator_id TEXT NOT NULL REFERENCES users(id),
  card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
  source_id TEXT NOT NULL,
  source_type TEXT NOT NULL DEFAULT 'Event',
  unread_count INTEGER NOT NULL DEFAULT 1,
  read_at TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(user_id,card_id)
);
CREATE INDEX IF NOT EXISTS notifications_user_read ON notifications(user_id,read_at,updated_at DESC);
CREATE INDEX IF NOT EXISTS notifications_source ON notifications(source_type,source_id);
CREATE TRIGGER IF NOT EXISTS notifications_event_delete AFTER DELETE ON events BEGIN
  DELETE FROM notifications WHERE source_type='Event' AND source_id=OLD.id;
END;
CREATE TRIGGER IF NOT EXISTS notifications_mention_delete AFTER DELETE ON mentions BEGIN
  DELETE FROM notifications WHERE source_type='Mention' AND source_id=OLD.id;
END;
CREATE TRIGGER IF NOT EXISTS mentions_card_delete AFTER DELETE ON cards BEGIN
  DELETE FROM mentions WHERE source_type='Card' AND source_id=OLD.id;
END;
CREATE TRIGGER IF NOT EXISTS mentions_comment_delete AFTER DELETE ON comments BEGIN
  DELETE FROM mentions WHERE source_type='Comment' AND source_id=OLD.id;
END;
CREATE TABLE IF NOT EXISTS notification_bundles (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  starts_at TEXT NOT NULL,
  ends_at TEXT NOT NULL,
  status INTEGER NOT NULL DEFAULT 0 CHECK(status IN (0,1,2)),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS notification_bundles_due ON notification_bundles(status,ends_at);
CREATE TABLE IF NOT EXISTS notification_settings (
  user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  bundle_email_frequency TEXT NOT NULL DEFAULT 'every_few_hours' CHECK(bundle_email_frequency IN ('never','every_few_hours','daily','weekly'))
);
CREATE TABLE IF NOT EXISTS notification_unsubscribe_tokens (
  user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  token TEXT NOT NULL UNIQUE,
  expires_at TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS notification_unsubscribe_tokens_expiry ON notification_unsubscribe_tokens(expires_at);
CREATE TABLE IF NOT EXISTS push_subscriptions (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  endpoint TEXT NOT NULL,
  p256dh_key TEXT,
  auth_key TEXT,
  user_agent TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(user_id,endpoint)
);
CREATE INDEX IF NOT EXISTS push_subscriptions_account ON push_subscriptions(account_id);
CREATE TABLE IF NOT EXISTS push_deliveries (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  notification_id TEXT NOT NULL REFERENCES notifications(id) ON DELETE CASCADE,
  source_id TEXT NOT NULL,
  status INTEGER NOT NULL DEFAULT 0 CHECK(status IN (0,1,2)),
  attempts INTEGER NOT NULL DEFAULT 0,
  next_attempt_at TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(notification_id,source_id)
);
CREATE INDEX IF NOT EXISTS push_deliveries_due ON push_deliveries(status,next_attempt_at);
CREATE TABLE IF NOT EXISTS push_delivery_receipts (
  delivery_id TEXT NOT NULL REFERENCES push_deliveries(id) ON DELETE CASCADE,
  subscription_id TEXT NOT NULL REFERENCES push_subscriptions(id) ON DELETE CASCADE,
  PRIMARY KEY(delivery_id,subscription_id)
);
CREATE TABLE IF NOT EXISTS user_timezones (
  user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  timezone_name TEXT
);
CREATE TABLE IF NOT EXISTS user_settings_records (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS user_settings_records_account ON user_settings_records(account_id);
CREATE TABLE IF NOT EXISTS filters (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  creator_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  fields TEXT NOT NULL DEFAULT '{}',
  params_digest TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(creator_id,params_digest)
);
CREATE INDEX IF NOT EXISTS filters_account_creator ON filters(account_id,creator_id);
CREATE TABLE IF NOT EXISTS filters_tags (
  filter_id TEXT NOT NULL REFERENCES filters(id) ON DELETE CASCADE,
  tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  PRIMARY KEY(filter_id,tag_id)
);
CREATE TABLE IF NOT EXISTS boards_filters (
  filter_id TEXT NOT NULL REFERENCES filters(id) ON DELETE CASCADE,
  board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  PRIMARY KEY(filter_id,board_id)
);
CREATE TABLE IF NOT EXISTS assignees_filters (
  filter_id TEXT NOT NULL REFERENCES filters(id) ON DELETE CASCADE,
  assignee_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  PRIMARY KEY(filter_id,assignee_id)
);
CREATE TABLE IF NOT EXISTS creators_filters (
  filter_id TEXT NOT NULL REFERENCES filters(id) ON DELETE CASCADE,
  creator_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  PRIMARY KEY(filter_id,creator_id)
);
CREATE TABLE IF NOT EXISTS closers_filters (
  filter_id TEXT NOT NULL REFERENCES filters(id) ON DELETE CASCADE,
  closer_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  PRIMARY KEY(filter_id,closer_id)
);
CREATE TABLE IF NOT EXISTS webhooks (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  url TEXT NOT NULL,
  active INTEGER NOT NULL DEFAULT 1,
  signing_secret TEXT NOT NULL,
  subscribed_actions TEXT NOT NULL DEFAULT '[]',
  consecutive_failures_count INTEGER NOT NULL DEFAULT 0,
  first_failure_at TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS webhooks_board_order ON webhooks(board_id,name,id DESC);
CREATE TABLE IF NOT EXISTS webhook_deliveries (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  webhook_id TEXT NOT NULL REFERENCES webhooks(id) ON DELETE CASCADE,
  event_id TEXT NOT NULL REFERENCES events(id) ON DELETE CASCADE,
  state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','in_progress','completed','errored')),
  request TEXT,
  response TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS webhook_deliveries_webhook_recent ON webhook_deliveries(webhook_id,created_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS webhook_deliveries_pending ON webhook_deliveries(state,created_at);
CREATE TRIGGER IF NOT EXISTS cards_touch_board_insert AFTER INSERT ON cards WHEN NEW.status='published' BEGIN
  UPDATE boards SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=NEW.board_id;
END;
CREATE TRIGGER IF NOT EXISTS cards_touch_board_update AFTER UPDATE ON cards WHEN NEW.status='published' AND NOT EXISTS(SELECT 1 FROM card_bulk_touches WHERE board_id=NEW.board_id) BEGIN
  UPDATE boards SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=NEW.board_id;
END;
CREATE VIRTUAL TABLE IF NOT EXISTS card_search USING fts5(title,description,tokenize='porter unicode61');
CREATE VIRTUAL TABLE IF NOT EXISTS comment_search USING fts5(body,tokenize='porter unicode61');
CREATE TABLE IF NOT EXISTS search_migrations (key TEXT PRIMARY KEY);
