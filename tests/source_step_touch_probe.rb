# Run with RAILS_ENV=test, DATABASE_URL pointing to a disposable Fizzy DB copy,
# and RIZZY_DISPOSABLE_SOURCE_DB=1. Prints only observed timestamp changes.
require "json"

raise "A disposable Fizzy test database is required" unless Rails.env.test? && ENV["RIZZY_DISPOSABLE_SOURCE_DB"] == "1"

account = Account.joins(:boards).first!
owner = account.users.find_by!(role: :owner)
board = account.boards.first!

def snapshot(card, board)
  [ card.reload.updated_at, board.reload.updated_at ]
end

def probe_steps(card, board)
  before = snapshot(card, board)
  sleep 0.02
  step = card.steps.create!(content: "Source step probe")
  created = snapshot(card, board)
  first_step_time = step.reload.updated_at
  sleep 0.02
  step.update!(content: "Source step probe edited")
  edited = snapshot(card, board)
  edited_step_time = step.reload.updated_at
  sleep 0.02
  step.update!(content: "Source step probe edited")
  repeated = snapshot(card, board)
  repeated_step_time = step.reload.updated_at
  sleep 0.02
  step.destroy!
  deleted = snapshot(card, board)
  {
    create_touches_card: created[0] > before[0],
    create_touches_board: created[1] > before[1],
    edit_updates_step: edited_step_time > first_step_time,
    edit_touches_card: edited[0] > created[0],
    edit_touches_board: edited[1] > created[1],
    repeat_updates_step: repeated_step_time > edited_step_time,
    repeat_touches_card: repeated[0] > edited[0],
    repeat_touches_board: repeated[1] > edited[1],
    delete_touches_card: deleted[0] > repeated[0],
    delete_touches_board: deleted[1] > repeated[1]
  }
end

result = Current.set(account: account, user: owner) do
  published = Card.create!(account: account, board: board, creator: owner, status: "published", title: "Published step probe")
  published_result = probe_steps(published, board)
  draft = Card.create!(account: account, board: board, creator: owner, status: "drafted", title: "Draft step probe")
  draft_result = probe_steps(draft, board)
  before_delete = board.reload.updated_at
  sleep 0.02
  published.destroy!
  { published: published_result, draft: draft_result,
    card_delete_touches_board: board.reload.updated_at > before_delete }
end
puts JSON.generate(result)
