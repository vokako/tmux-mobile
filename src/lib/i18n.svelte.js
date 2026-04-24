// Lightweight i18n with Svelte 5 reactivity — no dependencies

const i18n = $state({ lang: localStorage.getItem('tmux_locale') || (navigator.language?.startsWith('zh') ? 'zh' : 'en') });

export { i18n };

export function setLocale(l) {
  i18n.lang = l;
  localStorage.setItem('tmux_locale', l);
}

const msgs = {
  en: {
    sessions: 'Sessions',
    terminal: 'Terminal',
    chat: 'Chat',
    files: 'Files',

    theme: 'Theme',
    themeAuto: 'Auto',
    themeLight: 'Light',
    themeDark: 'Dark',
    font: 'Font',
    debug: 'Debug',
    on: 'On',
    off: 'Off',
    disconnect: 'Disconnect',
    sniff: 'Sniff',
    sniffing: 'Sniffing',
    language: 'Lang',

    connectTitle: 'Connect to your tmux server',
    address: 'Address',
    token: 'Token',
    tmuxSocket: 'tmux Socket',
    tmuxSocketHint: '(optional, -S path)',
    connect: 'Connect',
    connecting: 'Connecting…',
    cancel: 'Cancel',

    reconnecting: 'Reconnecting...',

    window: 'window',
    windows: 'windows',
    tapToKill: 'tap to kill',
    del: 'del',
    newSession: 'New Session',
    sessionName: 'Session name',
    workingDir: 'Working directory (optional)',
    commandOpt: 'Command (optional)',
    create: 'Create',
    noSubdirs: 'No subdirectories',

    noConversation: 'No conversation detected. Waiting for CLI output…',
    thinking: 'Thinking…',
    creatingSummary: 'Creating summary…',
    conversationSummary: 'Conversation Summary',
    selectModel: 'Select Model',
    copy: 'Copy',

    message: 'message…',

    loading: 'Loading...',
    emptyDir: 'Empty directory',
    noDownloads: 'No downloaded files',
    downloads: 'Downloads',
    folderName: 'folder name...',
    fileName: 'file name...',
    newName: 'new name...',
    discardChanges: 'Discard unsaved changes?',
    openFailed: 'Open failed: ',
    copied: 'Copied',
    selected: 'Selected — tap to copy',
    saved: 'Saved: ',
    open: 'Open',

    path: 'Path',
    type: 'Type',
    size: 'Size',
    modified: 'Modified',
    permissions: 'Permissions',
    readable: 'Readable',
    writable: 'Writable',
    textFile: 'Text file',
    yes: 'Yes',
    no: 'No',
    preview: 'Preview',

    addAll: 'Add All',
    commit: 'Commit',
    push: 'Push',
    status: 'Status',
    log: 'Log',
    commitMsg: 'commit message…',
    cleanTree: 'Working tree clean',
    gitLoading: 'Loading…',
  },
  zh: {
    sessions: '会 话',
    terminal: '终 端',
    chat: '聊 天',
    files: '文 件',

    theme: '主题',
    themeAuto: '自动',
    themeLight: '浅色',
    themeDark: '深色',
    font: '字号',
    debug: '调试',
    on: '开',
    off: '关',
    disconnect: '断开连接',
    sniff: '嗅探',
    sniffing: '嗅探中',
    language: '语言',

    connectTitle: '连接到 tmux 服务器',
    address: '地址',
    token: '令牌',
    tmuxSocket: 'tmux Socket',
    tmuxSocketHint: '(可选，-S 路径)',
    connect: '连接',
    connecting: '连接中…',
    cancel: '取消',

    reconnecting: '重新连接中...',

    window: '窗口',
    windows: '窗口',
    tapToKill: '点击删除',
    del: '删',
    newSession: '新建会话',
    sessionName: '会话名称',
    workingDir: '工作目录（可选）',
    commandOpt: '命令（可选）',
    create: '创建',
    noSubdirs: '没有子目录',

    noConversation: '未检测到对话，等待 CLI 输出…',
    thinking: '思考中…',
    creatingSummary: '生成摘要中…',
    conversationSummary: '对话摘要',
    selectModel: '选择模型',
    copy: '复制',

    message: '消息…',

    loading: '加载中...',
    emptyDir: '空目录',
    noDownloads: '没有已下载的文件',
    downloads: '下载',
    folderName: '文件夹名...',
    fileName: '文件名...',
    newName: '新名称...',
    discardChanges: '放弃未保存的更改？',
    openFailed: '打开失败：',
    copied: '已复制',
    selected: '已选中，点击复制',
    saved: '已保存：',
    open: '打开',

    path: '路径',
    type: '类型',
    size: '大小',
    modified: '修改时间',
    permissions: '权限',
    readable: '可读',
    writable: '可写',
    textFile: '文本文件',
    yes: '是',
    no: '否',
    preview: '预览',

    addAll: '全部添加',
    commit: '提交',
    push: '推送',
    status: '状态',
    log: '日志',
    commitMsg: '提交信息…',
    cleanTree: '工作区干净',
    gitLoading: '加载中…',
  },
};

export function t(key) {
  return msgs[i18n.lang]?.[key] ?? msgs.en[key] ?? key;
}
