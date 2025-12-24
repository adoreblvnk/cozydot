-- base config: https://wezterm.org/config/files.html
-- Nerd Font icon order: fa, md, oct, pl
local wezterm = require 'wezterm'
local act = wezterm.action
local config = wezterm.config_builder()

if wezterm.target_triple == 'x86_64-unknown-linux-gnu' then
  config.front_end = "Software"
elseif wezterm.target_triple == 'x86_64-pc-windows-msvc' then
  config.front_end = "Software" -- default 'WebGpu' but
  config.default_domain = 'WSL:Ubuntu'
end

-- Window Management
config.initial_cols = 120 -- default 120
config.initial_rows = 28 -- default 28
config.window_background_opacity = 0.80 -- default 1.0
config.window_decorations = "RESIZE" -- default "TITLE | RESIZE"
config.window_padding = { left = 8, right = 8, top = 8, bottom = 8 } -- default 0
config.adjust_window_size_when_changing_font_size = false -- default true

-- Styling
config.font = wezterm.font 'GeistMono Nerd Font'
config.color_scheme = 'Catppuccin Mocha'
local palettes = {
  ['Catppuccin Mocha'] = {
    rosewater = '#f5e0dc',
    pink      = '#f5c2e7',
    mauve     = '#cba6f7',
    red       = '#f38ba8',
    peach     = '#fab387',
    yellow    = '#f9e2af',
    green     = '#a6e3a1',
    sky       = '#89dceb',
    blue      = '#89b4fa',
    lavender  = '#b4befe',
    text      = '#cdd6f4',
    subtext0  = '#a6adc8',
    overlay0  = '#313244',
    base      = '#1e1e2e',
    mantle    = '#181825',
    crust     = '#11111b'
  },
  ['Catppuccin Latte'] = {
    rosewater = '#dc8a78',
    pink      = '#ea76cb',
    mauve     = '#8839ef',
    red       = '#d20f39',
    peach     = '#fe640b',
    yellow    = '#df8e1d',
    green     = '#40a02b',
    sky       = '#04a5e5',
    blue      = '#1e66f5',
    lavender  = '#7287fd',
    text      = '#4c4f69',
    subtext0  = '#6c6f85',
    overlay0  = '#9ca0b0',
    base      = '#eff1f5',
    mantle    = '#e6e9ef',
    crust     = '#dce0e8'
  },
}
-- fallback to Catppuccin Mocha
local colors = palettes[config.color_scheme] or palettes['Catppuccin Mocha']

-- Other Stuff
config.hyperlink_rules = wezterm.default_hyperlink_rules()

-- Cursor
config.default_cursor_style = 'BlinkingBlock'
config.cursor_blink_ease_in = 'Constant'
config.cursor_blink_ease_out = 'Constant'

-- Tab Bar
config.use_fancy_tab_bar = false -- default true, we set our own tab bar style
config.tab_max_width = 32
config.colors = { tab_bar = { background = colors.base } }
config.disable_default_key_bindings = true

wezterm.on('update-status', function(window, pane)
  local mode_text = "NORMAL"
  local mode_bg = colors.blue
  if window:active_key_table() then
    mode_text = window:active_key_table():upper():gsub("_MODE", "") -- converts "tab_mode" to "PANE"
    mode_bg = colors.mauve
  end
  -- left status for mode
  window:set_left_status(wezterm.format {
    { Attribute = { Intensity = 'Bold' } },
    { Background = { Color = mode_bg } },
    { Foreground = { Color = colors.base } },
    { Text = ' ' .. string.format("%-6s", mode_text) .. ' ' },
    { Background = { Color = colors.base } },
    { Foreground = { Color = mode_bg } },
    { Text = '' }, -- nf-pl-left_hard_divider
  })
  -- right status for workspace
  local cwd = ""
  if pane:get_current_working_dir() then
    -- convert Win paths to Unix style
    cwd = pane:get_current_working_dir().file_path:gsub("\\", "/")
  end
  -- replace home with ~ (replaces /home/user on WSL as well)
  cwd = cwd:gsub(wezterm.home_dir:gsub("\\", "/"), "~"):gsub("^/home/[^/]+", "~")
  -- match last 2 folders in path (eg converts ~/code/a/ to code/a)
  -- if no match, fallback to cwd
  cwd = cwd:match("([^/]+/[^/]+)/?$") or cwd
  window:set_right_status(wezterm.format {
    { Foreground = { Color = colors.peach } },
    { Attribute = { Intensity = 'Bold' } },
    { Text = '  ' .. cwd .. ' ' }, -- nf-fa-folder
    { Foreground = { Color = colors.mauve } },
    { Text = '|' },
    { Foreground = { Color = colors.peach } },
    { Attribute = { Intensity = 'Bold' } },
    { Text = '  ' .. window:active_workspace() .. ' ' }, -- nf-oct-terminal
  })
end)

-- tab title formatting
wezterm.on('format-tab-title', function(tab, tabs, panes, config, hover, max_width)
  local title = tab.active_pane.title -- title of active pane in tab
  -- use custom tab title if set (via tab:set_title())
  if tab.tab_title and #tab.tab_title > 0 then
    title = tab.tab_title
  end
  -- ensure title fits within max width
  title = wezterm.truncate_right(title, max_width - 4)
  local bg_color = colors.text
  if tab.is_active then
    bg_color = colors.green
  elseif hover then
    bg_color = colors.subtext0
  end
  return {
    { Background = { Color = bg_color } },
    { Foreground = { Color = colors.base } },
    { Text = '' },
    { Background = { Color = bg_color } },
    { Foreground = { Color = colors.mantle } },
    { Attribute = { Intensity = 'Bold' } },
    { Text = ' ' .. title .. ' ' },
    { Background = { Color = colors.base } },
    { Foreground = { Color = bg_color } },
    { Text = '' },
  }
end)

-- Key Bindings
config.keys = {
  -- some default keybinds for reference
  { key = 'l', mods = 'CTRL|SHIFT', action = act.ShowDebugOverlay },
  { key = 'p', mods = 'CTRL', action = wezterm.action.ActivateCommandPalette },
  { key = 'c', mods = 'CTRL|SHIFT', action = act.CopyTo 'Clipboard' },
  { key = 'v', mods = 'CTRL|SHIFT', action = act.PasteFrom 'Clipboard' },
  { key = 'f', mods = 'CTRL|SHIFT', action = act.Search 'CurrentSelectionOrEmptyString' },
  { key = 'x', mods = 'CTRL|SHIFT', action = act.ActivateCopyMode },
  -- font size
  { key = '+', mods = 'CTRL|SHIFT', action = act.IncreaseFontSize },
  { key = '_', mods = 'CTRL|SHIFT', action = act.DecreaseFontSize },
  { key = '0', mods = 'CTRL', action = act.ResetFontSize },
  -- scrolling
  { key = 'PageUp', mods = 'SHIFT', action = act.ScrollByPage(-1) },
  { key = 'PageDown', mods = 'SHIFT', action = act.ScrollByPage(1) },
  -- tab navigation
  { key = 'Tab', mods = 'CTRL', action = act.ActivateTabRelative(1) },
  { key = 'Tab', mods = 'CTRL|SHIFT', action = act.ActivateTabRelative(-1) },
  -- New Keybindings
  -- pane management
  { key = 'h', mods = 'ALT', action = act.ActivatePaneDirection 'Left' },
  { key = 'l', mods = 'ALT', action = act.ActivatePaneDirection 'Right' },
  { key = 'k', mods = 'ALT', action = act.ActivatePaneDirection 'Up' },
  { key = 'j', mods = 'ALT', action = act.ActivatePaneDirection 'Down' },
  { key = 'n', mods = 'ALT', action = act.SplitHorizontal { domain = 'CurrentPaneDomain' } },
  -- tab mode
  { key = 't', mods = 'CTRL', action = act.ActivateKeyTable { name = 'tab_mode' }},
  -- pane mode
  { key = 'p', mods = 'CTRL', action = act.ActivateKeyTable { name = 'pane_mode' } },
  -- resize mode
  { key = 'n', mods = 'CTRL', action = act.ActivateKeyTable { name = 'resize_mode', one_shot = false } },
  -- move mode
  { key = 'h', mods = 'CTRL', action = act.ActivateKeyTable { name = 'move_mode' }},
}

local key_tables = {
  tab_mode = {
    -- actions
    { key = 'n', action = act.SpawnTab 'CurrentPaneDomain' },
    { key = 'x', action = act.CloseCurrentTab { confirm = false } },
    -- navigation
    { key = 'h', action = act.ActivateTabRelative(-1) },
    { key = 'l', action = act.ActivateTabRelative(1) },
    { key = '1', action = act.ActivateTab(0) },
    { key = '2', action = act.ActivateTab(1) },
    { key = '3', action = act.ActivateTab(2) },
    { key = '4', action = act.ActivateTab(3) },
    -- rename
    { key = 'r',
      action = act.PromptInputLine {
        description = 'Enter new name for tab',
        action = wezterm.action_callback(function(window, pane, line)
          if line then window:active_tab():set_title(line) end
        end),
      },
    },
    { key = 'q', action = 'PopKeyTable' },
    { key = 'Escape', action = 'PopKeyTable' },
    { key = 't', mods = 'CTRL', action = 'PopKeyTable' },
  },
  pane_mode = {
    -- actions
    { key = 'n', action = act.SplitHorizontal { domain = 'CurrentPaneDomain' } },
    { key = 'r', action = act.SplitHorizontal { domain = 'CurrentPaneDomain' } },
    { key = 'd', action = act.SplitVertical { domain = 'CurrentPaneDomain' } },
    { key = 'x', action = act.CloseCurrentPane { confirm = false } },
    { key = 'f', action = act.TogglePaneZoomState },
    -- navigation
    { key = 'h', action = act.ActivatePaneDirection 'Left' },
    { key = 'j', action = act.ActivatePaneDirection 'Down' },
    { key = 'k', action = act.ActivatePaneDirection 'Up' },
    { key = 'l', action = act.ActivatePaneDirection 'Right' },
    { key = 'p', action = act.ActivatePaneDirection 'Next' }, -- next pane
    { key = 'q', action = 'PopKeyTable' },
    { key = 'Escape', action = 'PopKeyTable' },
    { key = 'p', mods = 'CTRL', action = 'PopKeyTable' },
  },
  resize_mode = {
    { key = 'h', action = act.AdjustPaneSize { 'Left', 5 } },
    { key = 'j', action = act.AdjustPaneSize { 'Down', 5 } },
    { key = 'k', action = act.AdjustPaneSize { 'Up', 5 } },
    { key = 'l', action = act.AdjustPaneSize { 'Right', 5 } },
    { key = 'n', mods = 'CTRL', action = 'PopKeyTable' },
  },
  move_mode = {
    { key = 'n', action = act.RotatePanes 'Clockwise' },
    { key = 'p', action = act.RotatePanes 'CounterClockwise' },
    { key = 'h', mods = 'CTRL', action = 'PopKeyTable' },
  },
}

-- global exit keys for key tables
local common_keys = {
  { key = 'q', action = 'PopKeyTable' },
  { key = 'Escape', action = 'PopKeyTable' },
  { key = 'Enter', action = 'PopKeyTable' }
}
for mode, tables in pairs(key_tables) do
  for _, key_obj in ipairs(common_keys) do
    table.insert(tables, key_obj)
  end
end
config.key_tables = key_tables -- set config.key_tables

-- Mouse Bindings
config.mouse_bindings = {
  {
    event = { Up = { streak = 1, button = 'Left' } },
    mods = 'CTRL',
    action = act.OpenLinkAtMouseCursor
  },
  {
    event = { Down = { streak = 1, button = 'Right' } },
    action = act.PasteFrom 'Clipboard'
  },
}

return config
