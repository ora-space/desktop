Ora 是一个专为 AI Agent 设计的“研发底座”，基于 Tauri 2.0 构建，集成各类主流 Agent CLI 的操作环境。

前端开发基于 shadcn，配色参考 shadcn。

## 顶部标题栏

App 顶部是自己实现的窗口标题栏区域（布局上划分，实际背景色和 App 主体无异），以取代 Windows 原生标题栏。标题栏左侧是软件圆角图标（以纯黑色替代），中间是拖动窗口的网点 handle，只有鼠标悬浮时展示（在设计稿中要显示）。右侧是交通灯平放的最小化、放大、关闭按钮，要求三个按钮后方有接近满圆角矩形背景。

## 左侧项目切换栏

App 左侧是切换不同项目的 Sidebar，最上面是 Ellipsis Icon，往下是 Plus Button，再往下是一个短横线隔开。下方是切换不同项目的按钮。

在切换栏最下面是 Avatar Icon。

## 主体

除开边框，就是 App 的主体部分。主体部分是 Side Panel + Content 的结构。

### Side Panel

顶部是 Brand Icon + "Ora Desktop" 左对齐，收起 panel 按钮右对齐。

下方是 "Agent Task" 消息列表
