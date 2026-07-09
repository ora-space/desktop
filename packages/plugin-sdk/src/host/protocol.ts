/** Host → Plugin 的请求（从 stdin 读取） */
export interface HostRequest {
  id: number;
  method: string;
  params: any;
}

/** Plugin → Host 的成功响应 */
export interface SuccessResponse {
  jsonrpc: "2.0";
  id: number;
  result: any;
}

/** Plugin → Host 的错误响应 */
export interface ErrorResponse {
  jsonrpc: "2.0";
  id: number;
  error: {
    code: number;
    message: string;
  };
}
